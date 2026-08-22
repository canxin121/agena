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
import { getToolPartDetail, type ToolDetailSection } from '@/stores/chat/api'
import type { JsonValue } from '@/types/json'
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

const directory = useDirectoryStore()
const ui = useUiStore()

const metadataExpanded = ref(false)
const inputExpanded = ref(false)
const outputExpanded = ref(false)
const outputMetadataExpanded = ref(false)
const presentationExpanded = ref(true)
const sectionValues = ref<Partial<Record<ToolDetailSection, JsonValue>>>({})
const loadingSections = ref<Set<ToolDetailSection>>(new Set())
const sectionErrors = ref<Partial<Record<ToolDetailSection, string>>>({})
const loadedPartKey = ref('')
const toolDetailSections: ToolDetailSection[] = ['metadata', 'input', 'output', 'output_metadata', 'presentation']

const operation = computed(() => operationPresentation(props.part, sectionValues.value))
const status = computed(() => partStatusPresentation(props.part.status))

function sectionLoaded(section: ToolDetailSection): boolean {
  return Object.prototype.hasOwnProperty.call(sectionValues.value, section)
}

function sectionLoading(section: ToolDetailSection): boolean {
  return loadingSections.value.has(section)
}

function sectionError(section: ToolDetailSection): string {
  return sectionErrors.value[section] || ''
}

function sectionExpanded(section: ToolDetailSection): boolean {
  if (section === 'metadata') return metadataExpanded.value
  if (section === 'input') return inputExpanded.value
  if (section === 'output') return outputExpanded.value
  if (section === 'output_metadata') return outputMetadataExpanded.value
  return presentationExpanded.value
}

function setSectionExpanded(section: ToolDetailSection, expanded: boolean) {
  if (section === 'metadata') metadataExpanded.value = expanded
  else if (section === 'input') inputExpanded.value = expanded
  else if (section === 'output') outputExpanded.value = expanded
  else if (section === 'output_metadata') outputMetadataExpanded.value = expanded
  else presentationExpanded.value = expanded
}

async function loadSection(section: ToolDetailSection) {
  // Presentation is part of every transcript snapshot. The other sections
  // are deliberately fetched only after their disclosure row is opened.
  if (section === 'presentation' || sectionLoaded(section) || sectionLoading(section)) return
  const sessionId = String(props.sessionId || '').trim()
  const partId = String(props.part.id || '').trim()
  if (!sessionId || !partId) return

  loadingSections.value = new Set([...loadingSections.value, section])
  sectionErrors.value = { ...sectionErrors.value, [section]: '' }
  try {
    const resource = await getToolPartDetail(sessionId, partId, section)
    if (resource.part_id !== Number(partId) || resource.section !== section) {
      throw new Error('The server returned a mismatched tool detail section')
    }
    sectionValues.value = { ...sectionValues.value, [section]: resource.value }
  } catch (error) {
    sectionErrors.value = {
      ...sectionErrors.value,
      [section]: error instanceof Error ? error.message : 'Unable to load this section',
    }
  } finally {
    const next = new Set(loadingSections.value)
    next.delete(section)
    loadingSections.value = next
  }
}

async function toggleSection(section: ToolDetailSection) {
  emit('select')
  const expanded = !sectionExpanded(section)
  setSectionExpanded(section, expanded)
  if (expanded) await loadSection(section)
}

function resetSectionState() {
  metadataExpanded.value = false
  inputExpanded.value = false
  outputExpanded.value = false
  outputMetadataExpanded.value = false
  // Presentation is the human-readable primary view and is open by default.
  presentationExpanded.value = true
}

watch(
  () => `${props.sessionId || ''}:${props.part.id || ''}`,
  (key) => {
    if (loadedPartKey.value && loadedPartKey.value !== key) {
      sectionValues.value = {}
      loadingSections.value = new Set()
      sectionErrors.value = {}
    }
    loadedPartKey.value = key
    resetSectionState()
  },
  { immediate: true },
)

watch(
  () => props.collapseSignal,
  () => resetSectionState(),
)

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
      <span class="min-w-0 flex-1 truncate text-[13px] font-semibold">{{
        operation.userInputs[0] ? operation.toolName || operation.title : operation.title
      }}</span>
      <span
        v-if="!expanded && (operation.userInputs[0]?.questions[0]?.question || operation.summary)"
        class="min-w-0 max-w-[55%] truncate text-xs text-muted-foreground"
      >
        · {{ operation.userInputs[0]?.questions[0]?.question || operation.userInputs[0]?.title || operation.summary }}
      </span>
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

      <section
        v-for="section in toolDetailSections"
        :key="section"
        class="py-1"
      >
        <button
          type="button"
          class="flex items-center gap-2 rounded-md px-1 py-1 text-xs font-semibold text-primary outline-none hover:bg-muted/40"
          :aria-expanded="sectionExpanded(section)"
          :data-tool-detail-section="section"
          @click="toggleSection(section)"
        >
          <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
            sectionExpanded(section) ? '▾' : '▸'
          }}</span>
          {{ section === 'output_metadata' ? 'Output metadata' : section[0].toUpperCase() + section.slice(1) }}
          <span v-if="sectionLoading(section)" class="font-normal text-muted-foreground">Loading…</span>
        </button>

        <div v-if="sectionExpanded(section)" class="min-w-0 pl-5 pt-1">
          <div v-if="sectionError(section)" class="py-1 text-xs text-rose-700 dark:text-rose-300">
            {{ sectionError(section) }}
          </div>

          <template v-else-if="section === 'metadata'">
            <MarkdownRenderer :content="structuredValueMarkdown(operation.metadata)" mode="markdown" :stream="false" />
          </template>

          <template v-else-if="section === 'input'">
            <MarkdownRenderer
              v-if="operation.inputMarkdown"
              :content="operation.inputMarkdown"
              mode="markdown"
              :stream="false"
            />
            <CodeBlock v-else :code="prettyJson(operation.input || {})" lang="json" compact />
          </template>

          <template v-else-if="section === 'output'">
            <MarkdownRenderer v-if="operation.outputText" :content="operation.outputText" mode="markdown" :stream="false" />
            <CodeBlock
              v-if="operation.structured !== null"
              :code="prettyJson(operation.structured)"
              lang="json"
              compact
            />
            <CodeBlock
              v-else-if="operation.rawOutput !== null && !operation.outputText"
              :code="prettyJson(operation.rawOutput)"
              lang="json"
              compact
            />
            <div v-if="operation.managedOutputs !== null" class="mt-2">
              <div class="mb-1 text-[11px] font-semibold text-muted-foreground">Managed outputs</div>
              <CodeBlock :code="prettyJson(operation.managedOutputs)" lang="json" compact />
            </div>
            <div v-if="operation.truncated" class="mt-1 text-[11px] text-muted-foreground">Output truncated.</div>
            <div v-if="operation.attachments.length" class="mt-3 space-y-1">
              <AgenaAttachmentPreview
                v-for="attachment in operation.attachments"
                :key="attachment.key"
                :attachment="attachment"
                :workspace-root="String(directory.currentDirectory || '')"
                @open="openAttachment"
              />
            </div>
          </template>

          <template v-else-if="section === 'output_metadata'">
            <MarkdownRenderer
              :content="structuredValueMarkdown(operation.outputMetadata)"
              mode="markdown"
              :stream="false"
            />
          </template>

          <template v-else>
            <MarkdownRenderer
              v-if="operation.summary"
              :content="operation.summary"
              mode="markdown"
              :stream="false"
            />
            <div v-if="operation.presentationBlocks.length" class="mt-2 space-y-3">
              <AgenaOperationBlock
                v-for="(block, index) in operation.presentationBlocks"
                :key="String(block.id || `${block.type || block.kind || 'block'}:${index}`)"
                :block="block"
              />
            </div>
            <div
              v-if="!operation.summary && !operation.presentationBlocks.length"
              class="text-xs text-muted-foreground"
            >
              No presentation details.
            </div>
          </template>
        </div>
      </section>
    </div>
  </div>
</template>
