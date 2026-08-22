<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import AgenaAttachmentPreview from '@/components/chat/AgenaAttachmentPreview.vue'
import AgenaInteractionPart from '@/components/chat/AgenaInteractionPart.vue'
import AgenaOperationBlock from '@/components/chat/AgenaOperationBlock.vue'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import {
  operationPresentation,
  partStatusPresentation,
  prettyJson,
  structuredValueMarkdown,
} from '@/pages/chat/transcriptPartPresentation'
import { useDirectoryStore } from '@/stores/directory'
import { useUiStore } from '@/stores/ui'

const props = defineProps<{
  part: TranscriptDisplayPart
  expanded: boolean
  collapseSignal: number
  sessionId?: string | null
}>()

const emit = defineEmits<{
  (event: 'toggle'): void
  (event: 'select'): void
}>()

const operation = computed(() => operationPresentation(props.part))
const status = computed(() => partStatusPresentation(props.part.status))
const directory = useDirectoryStore()
const ui = useUiStore()
const inputExpanded = ref(false)
const outputExpanded = ref(false)
const stdoutExpanded = ref(true)
const metadataExpanded = ref(false)

watch(
  () => props.collapseSignal,
  () => {
    inputExpanded.value = false
    outputExpanded.value = false
    stdoutExpanded.value = true
    metadataExpanded.value = false
  },
)

const primaryInteraction = computed(() => operation.value.userInputs[0] || null)
const headlineTitle = computed(() =>
  primaryInteraction.value ? operation.value.toolName || operation.value.title : operation.value.title,
)
const headlineSummary = computed(
  () => primaryInteraction.value?.questions[0]?.question || primaryInteraction.value?.title || operation.value.summary,
)
const hasOutput = computed(() => {
  const value = operation.value
  return Boolean(value.structured !== null || value.blocks.length || value.attachments.length)
})
const hasMetadata = computed(() => Object.keys(operation.value.metadata).length > 0)

function openAttachment(path: string, url: string) {
  const workspace = String(directory.currentDirectory || '').trim()
  if (workspace && path) {
    ui.requestWorkspaceDockFile(path, 'open')
    return
  }
  if (url) window.open(url, '_blank', 'noopener,noreferrer')
}

function toggleOuter() {
  emit('select')
  emit('toggle')
}
</script>

<template>
  <div class="min-w-0">
    <button
      type="button"
      class="group/headline flex w-full min-w-0 items-baseline gap-2 rounded-md px-1.5 py-1 text-left outline-none hover:bg-muted/35 focus-visible:ring-1 focus-visible:ring-ring/50"
      :aria-expanded="expanded"
      data-transcript-vim-toggle="true"
      @click="toggleOuter"
      @focus="$emit('select')"
    >
      <span class="w-3 shrink-0 text-center font-mono text-xs text-muted-foreground" aria-hidden="true">{{
        expanded ? '▾' : '▸'
      }}</span>
      <span
        class="w-3 shrink-0 text-center font-mono text-xs"
        :class="{
          'text-primary': status.tone === 'pending',
          'text-emerald-600 dark:text-emerald-400': status.tone === 'success',
          'text-amber-600 dark:text-amber-400': status.tone === 'warning',
          'text-rose-600 dark:text-rose-400': status.tone === 'danger',
          'text-muted-foreground': status.tone === 'muted',
          'animate-spin': status.spinning,
        }"
        :title="status.label"
        aria-hidden="true"
        >{{ status.icon }}</span
      >
      <span class="min-w-0 flex-1 truncate text-[13px] font-semibold">{{ headlineTitle }}</span>
      <span v-if="!expanded && headlineSummary" class="min-w-0 max-w-[55%] truncate text-xs text-muted-foreground"
        >· {{ headlineSummary }}</span
      >
      <span v-if="operation.durationMs !== null" class="shrink-0 font-mono text-[10px] text-muted-foreground/70">
        {{ operation.durationMs }}ms
      </span>
    </button>

    <div v-if="expanded" class="ml-5 rounded-r-md border-l border-border/60 bg-muted/[0.08] pb-1 pl-4 pr-1">
      <div v-if="operation.userInputs.length" class="space-y-4 py-1 text-sm">
        <AgenaInteractionPart
          v-for="interaction in operation.userInputs"
          :key="interaction.requestId || interaction.title"
          :interaction="interaction"
          :session-id="sessionId"
        />
      </div>

      <section v-if="operation.permissions.length" class="space-y-4 py-1 text-sm">
        <AgenaInteractionPart
          v-for="permission in operation.permissions"
          :key="permission.requestId || `${permission.action}:${permission.status}`"
          :permission="permission"
          :session-id="sessionId"
        />
      </section>

      <section v-if="operation.error" class="py-1.5">
        <div class="text-xs font-semibold text-rose-600 dark:text-rose-400">› Error</div>
        <pre
          class="mt-1 whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-rose-700 dark:text-rose-300"
          >{{ operation.error }}</pre
        >
      </section>

      <section v-if="operation.input !== null" class="py-1">
        <button
          type="button"
          class="flex items-center gap-2 rounded-md px-1 py-1 text-xs font-semibold text-primary outline-none hover:bg-muted/40"
          :aria-expanded="inputExpanded"
          @click="inputExpanded = !inputExpanded"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            inputExpanded ? '▾' : '▸'
          }}</span>
          Input
        </button>
        <div v-if="inputExpanded" class="pl-5 pt-1">
          <MarkdownRenderer
            v-if="operation.inputMarkdown"
            :content="operation.inputMarkdown"
            mode="markdown"
            :stream="false"
          />
          <CodeBlock v-else :code="prettyJson(operation.input)" lang="json" compact />
        </div>
      </section>

      <section v-if="hasOutput" class="py-1">
        <button
          type="button"
          class="flex items-center gap-2 rounded-md px-1 py-1 text-xs font-semibold text-primary outline-none hover:bg-muted/40"
          :aria-expanded="outputExpanded"
          @click="outputExpanded = !outputExpanded"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            outputExpanded ? '▾' : '▸'
          }}</span>
          Output
        </button>

        <div v-if="outputExpanded" class="space-y-3 pl-5 pt-1">
          <AgenaOperationBlock
            v-for="(block, index) in operation.blocks"
            :key="String(block.id || `${block.type || block.kind || 'block'}:${index}`)"
            :block="block"
          />

          <div v-if="operation.attachments.length" class="space-y-1">
            <AgenaAttachmentPreview
              v-for="attachment in operation.attachments"
              :key="attachment.key"
              :attachment="attachment"
              :workspace-root="String(directory.currentDirectory || '')"
              @open="openAttachment"
            />
          </div>

          <CodeBlock
            v-if="operation.structured !== null && !operation.blocks.length"
            :code="prettyJson(operation.structured)"
            lang="json"
            compact
          />
        </div>
      </section>

      <section v-if="hasMetadata" class="py-1">
        <button
          type="button"
          class="flex items-center gap-2 rounded-md px-1 py-1 text-xs font-semibold text-primary outline-none hover:bg-muted/40"
          :aria-expanded="metadataExpanded"
          @click="metadataExpanded = !metadataExpanded"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            metadataExpanded ? '▾' : '▸'
          }}</span>
          Metadata
        </button>
        <div v-if="metadataExpanded" class="pl-5 pt-1">
          <MarkdownRenderer :content="structuredValueMarkdown(operation.metadata)" mode="markdown" :stream="false" />
        </div>
      </section>

      <section v-if="operation.stdout" class="py-1">
        <button
          type="button"
          class="flex items-center gap-2 rounded-md px-1 py-1 text-xs font-semibold text-primary outline-none hover:bg-muted/40"
          :aria-expanded="stdoutExpanded"
          @click="stdoutExpanded = !stdoutExpanded"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            stdoutExpanded ? '▾' : '▸'
          }}</span>
          Stdout
        </button>
        <div v-if="stdoutExpanded" class="min-w-0 pl-5 pt-1">
          <MarkdownRenderer :content="operation.stdout" mode="markdown" :stream="false" />
        </div>
      </section>
    </div>
  </div>
</template>
