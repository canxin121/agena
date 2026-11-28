<script setup lang="ts">
import { onBeforeUnmount, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '@/components/ui/Button.vue'
import { downloadBlob } from '@/lib/authenticatedResources'
import { mcpAppResourceDownload, type McpAppDownload } from './bridge'

const props = defineProps<{
  files: McpAppDownload[]
  read: (uri: string, signal: AbortSignal) => Promise<unknown>
}>()
const emit = defineEmits<{ cancel: []; complete: []; failure: [message: string] }>()
const { t } = useI18n()
type PreparedFile = Extract<McpAppDownload, { blob: Blob }>
const inlineFiles = props.files.filter((file) => 'blob' in file)
const prepared = shallowRef<PreparedFile[] | null>(inlineFiles.length === props.files.length ? inlineFiles : null)
const preparing = shallowRef<AbortController>()

function stopPreparing() {
  preparing.value?.abort()
  preparing.value = undefined
}
onBeforeUnmount(stopPreparing)

function cancel() {
  stopPreparing()
  emit('cancel')
}

async function prepare() {
  if (preparing.value) return
  const controller = new AbortController()
  preparing.value = controller
  const deadline = setTimeout(() => controller.abort(), 45_000)
  try {
    const files: PreparedFile[] = []
    for (const file of props.files) {
      if ('blob' in file) {
        files.push(file)
        continue
      }
      const result = await props.read(file.uri, controller.signal)
      controller.signal.throwIfAborted()
      const blob = mcpAppResourceDownload(result, file.uri)
      if (!blob) throw new Error(t('chat.mcpApp.downloadUnavailable'))
      files.push({ filename: file.filename, blob })
    }
    if (preparing.value === controller) prepared.value = files
  } catch (error) {
    if (preparing.value === controller) {
      emit('failure', error instanceof Error ? error.message : t('chat.mcpApp.downloadUnavailable'))
    }
  } finally {
    clearTimeout(deadline)
    if (preparing.value === controller) preparing.value = undefined
  }
}

function save(file: PreparedFile) {
  if (!prepared.value?.includes(file)) return
  try {
    downloadBlob(file.blob, file.filename)
    prepared.value = prepared.value.filter((pending) => pending !== file)
    if (!prepared.value.length) emit('complete')
  } catch {
    emit('failure', t('chat.mcpApp.downloadUnavailable'))
  }
}
</script>

<template>
  <div class="shrink-0 border-t px-3 py-2 text-xs" role="group">
    <p class="font-medium">{{ t('chat.mcpApp.actions.download') }}</p>
    <ul v-if="!prepared" class="mt-1 max-h-36 list-disc space-y-1 overflow-auto pl-4">
      <li v-for="(file, index) in files" :key="index" class="break-all">{{ file.filename }}</li>
    </ul>
    <div class="mt-2 flex flex-wrap items-center gap-2">
      <template v-if="prepared">
        <Button
          v-for="(file, index) in prepared"
          :key="index"
          size="sm"
          class="max-w-full"
          :title="file.filename"
          @click="save(file)"
        >
          <span class="truncate">{{ t('chat.mcpApp.downloadFile', { filename: file.filename }) }}</span>
        </Button>
      </template>
      <Button v-else size="sm" :disabled="!!preparing" @click="prepare">
        {{ t(preparing ? 'chat.mcpApp.preparingDownloads' : 'chat.mcpApp.prepareDownloads') }}
      </Button>
      <Button variant="ghost" size="sm" @click="cancel">{{ t('common.cancel') }}</Button>
    </div>
  </div>
</template>
