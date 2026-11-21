<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiPlugLine, RiTerminalBoxLine } from '@remixicon/vue'
import type { ComposerProcess } from '@/pages/chat/sessionProcesses'
import { projectLocalPart } from '@/pages/chat/transcriptProjection'
import { partStatusPresentation } from '@/pages/chat/transcriptPartPresentation'
import ProcessControls from './ProcessControls.vue'
import SessionChip from './SessionChip.vue'

const AgenaContentOutput = defineAsyncComponent(() => import('./AgenaContentOutput.vue'))
const AgenaOperationPart = defineAsyncComponent(() => import('./AgenaOperationPart.vue'))
const props = defineProps<{ sessionId: string; entry: ComposerProcess }>()
const emit = defineEmits<{ changed: [] }>()
const { t } = useI18n()
const open = ref(false)
const expanded = ref(true)
function setOpen(value: boolean) {
  open.value = value
  expanded.value = true
}
const typeLabel = computed(() =>
  t(props.entry.kind === 'shell' ? 'chat.operations.process' : 'chat.operations.subscription'),
)
const part = computed(() => props.entry.part && projectLocalPart(props.entry.part, 'assistant'))
const status = computed(() => {
  const { process, exitCode, kind, part } = props.entry
  const code = process?.exit_code ?? exitCode
  if (kind === 'shell' && code != null) return t('chat.activityLog.exitCode', { code })
  if (process)
    return t(
      process.ready && process.status === 'running'
        ? 'chat.operations.ready'
        : `chat.operations.status.${process.status}`,
    )
  if (props.entry.retained) return t('chat.toolDetails.output')
  return partStatusPresentation(part?.partState || '').label
})
</script>

<template>
  <SessionChip
    class="composer-resource"
    :title="typeLabel"
    :hint="`${typeLabel} · ${entry.label} · ${status}`"
    :open="open"
    :data-process-chip="entry.kind"
    @update:open="setOpen"
  >
    <template #trigger>
      <component
        :is="entry.kind === 'shell' ? RiTerminalBoxLine : RiPlugLine"
        class="resource-icon"
        aria-hidden="true"
      />
      <span class="min-w-0 text-start">
        <span class="resource-name" :class="{ 'font-mono': entry.kind === 'shell' }">{{ entry.label }}</span>
        <span class="resource-caption">{{ status }}</span>
      </span>
    </template>
    <template v-if="entry.process && (entry.process.output_resource || !part)">
      <pre class="mb-2 whitespace-pre-wrap break-words font-mono text-xs" dir="auto">{{ entry.process.command }}</pre>
      <AgenaContentOutput
        v-if="entry.process.output_resource"
        :key="entry.process.output_resource.resource_id"
        :resource="entry.process.output_resource"
        :session-id="sessionId"
      />
      <ProcessControls
        :session-id="sessionId"
        :kind="entry.kind"
        :id="entry.process.process_id"
        @changed="emit('changed')"
      />
    </template>
    <AgenaOperationPart
      v-else-if="part"
      :part="part"
      :session-id="sessionId"
      :expanded="expanded"
      :collapse-signal="0"
      @toggle="expanded = !expanded"
    />
  </SessionChip>
</template>
