<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowLeftLine, RiArrowRightSLine, RiPlugLine, RiRefreshLine, RiTerminalBoxLine } from '@remixicon/vue'
import { apiJson } from '@/lib/api'
import type { MessageEntry } from '@/types/chat'
import {
  composerProcesses,
  processCollection,
  type ComposerProcess,
  type ProcessKind,
  type SessionProcess,
} from '@/pages/chat/sessionProcesses'
import { useVisibleResource } from '@/pages/chat/useVisibleResource'
import { projectLocalPart } from '@/pages/chat/transcriptProjection'
import { partStatusPresentation } from '@/pages/chat/transcriptPartPresentation'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import ProcessControls from './ProcessControls.vue'
import SessionChip from './SessionChip.vue'

const AgenaContentOutput = defineAsyncComponent(() => import('./AgenaContentOutput.vue'))
const AgenaOperationPart = defineAsyncComponent(() => import('./AgenaOperationPart.vue'))
const props = defineProps<{ sessionId: string; messages: MessageEntry[]; busy: boolean }>()
const { t } = useI18n()
const open = ref(false)
const selectedKey = ref<string>()
const expanded = ref(true)
const session = computed(() => props.sessionId)

// Managed processes can outlive the launch part's loaded transcript window.
function processList(kind: ProcessKind) {
  const reader = useVisibleResource<SessionProcess[]>({
    key: session,
    minInterval: 1000,
    interval: (): number => (props.busy || reader.data.value?.some((row) => row.status === 'running') ? 3000 : 30000),
    load: (id, signal) => apiJson<SessionProcess[]>(processCollection(id, kind), { signal, cache: 'no-store' }),
  })
  return reader
}
const shells = processList('shell')
const monitors = processList('monitor')
const processes = computed(() =>
  composerProcesses(props.sessionId, props.messages, shells.data.value ?? [], monitors.data.value ?? []),
)
const selected = computed(() => processes.value.find((entry) => entry.key === selectedKey.value))
const part = computed(() => selected.value?.part && projectLocalPart(selected.value.part, 'assistant'))
const loading = computed(() => shells.loading.value || monitors.loading.value)
const errors = computed(() =>
  [
    { label: t('chat.operations.process'), error: shells.error.value },
    { label: t('chat.operations.subscription'), error: monitors.error.value },
  ].filter((entry) => entry.error),
)
const title = computed(() => t('nav.terminal'))
const label = computed(() => (processes.value.length ? `${title.value} · ${processes.value.length}` : title.value))

function select(key?: string) {
  selectedKey.value = key
  expanded.value = true
}
function status(entry: ComposerProcess) {
  const { process, exitCode, kind, part } = entry
  const code = process?.exit_code ?? exitCode
  if (kind === 'shell' && code != null) return t('chat.activityLog.exitCode', { code })
  if (process)
    return t(
      process.ready && process.status === 'running'
        ? 'chat.operations.ready'
        : `chat.operations.status.${process.status}`,
    )
  if (entry.retained) return t('chat.toolDetails.output')
  return partStatusPresentation(part?.partState || '').label
}
function refresh() {
  void shells.refresh()
  void monitors.refresh()
}
watch(() => props.busy, refresh)
watch(open, (value) => {
  if (value) refresh()
  else select()
})
</script>

<template>
  <SessionChip
    v-if="open || processes.length || errors.length"
    data-terminal-chip
    :title="selected?.label || title"
    :label="label"
    :hint="label"
    :open="open"
    @update:open="open = $event"
  >
    <template #icon>
      <RiTerminalBoxLine class="h-3 w-3 shrink-0" aria-hidden="true" />
    </template>
    <template #actions>
      <Button v-if="selected" size="sm" variant="ghost" @click="select()">
        <RiArrowLeftLine aria-hidden="true" />{{ t('common.back') }}
      </Button>
      <IconButton
        v-else
        class="h-7 w-7"
        :tooltip="t('common.refresh')"
        :aria-label="t('common.refresh')"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <template v-if="selected">
      <template v-if="selected.process && (selected.process.output_resource || !part)">
        <pre class="mb-2 whitespace-pre-wrap break-words font-mono text-xs" dir="auto">{{
          selected.process.command
        }}</pre>
        <AgenaContentOutput
          v-if="selected.process.output_resource"
          :key="selected.process.output_resource.resource_id"
          :resource="selected.process.output_resource"
          :session-id="sessionId"
        />
        <ProcessControls
          :key="selected.key"
          :session-id="sessionId"
          :kind="selected.kind"
          :id="selected.process.process_id"
          @changed="(selected.kind === 'shell' ? shells : monitors).refresh()"
        />
      </template>
      <AgenaOperationPart
        v-else-if="part"
        :key="part.id"
        :part="part"
        :session-id="sessionId"
        :expanded="expanded"
        :collapse-signal="0"
        @toggle="expanded = !expanded"
      />
    </template>
    <template v-else>
      <p v-for="item in errors" :key="item.label" role="alert" class="mb-1 break-words text-xs text-destructive">
        {{ item.label }} · {{ item.error }}
      </p>
      <p v-if="!processes.length && !errors.length" class="text-xs text-muted-foreground">
        {{ t(loading ? 'common.loading' : 'chat.operations.noProcesses') }}
      </p>
      <ul v-if="processes.length" class="space-y-0.5">
        <li v-for="entry in processes" :key="entry.key">
          <button
            type="button"
            class="flex min-h-8 w-full min-w-0 items-center gap-2 rounded px-2 text-start text-xs hover:bg-secondary/50 focus-visible:ring-2 focus-visible:ring-ring"
            :title="entry.label"
            @click="select(entry.key)"
          >
            <component
              :is="entry.kind === 'shell' ? RiTerminalBoxLine : RiPlugLine"
              class="h-4 w-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <span class="min-w-0 flex-1 truncate" :class="{ 'font-mono': entry.kind === 'shell' }" dir="auto">
              {{ entry.label }}
            </span>
            <span class="shrink-0 text-muted-foreground">{{ status(entry) }}</span>
            <RiArrowRightSLine class="h-4 w-4 shrink-0 text-muted-foreground rtl:rotate-180" aria-hidden="true" />
          </button>
        </li>
      </ul>
    </template>
  </SessionChip>
</template>
