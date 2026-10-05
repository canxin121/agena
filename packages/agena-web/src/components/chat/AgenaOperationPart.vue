<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
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

const { t } = useI18n()
const detailsExpanded = ref(false)
const metadataExpanded = ref(false)
const inputExpanded = ref(false)
const outputExpanded = ref(false)
const presentationExpanded = ref(false)
let partGeneration = 0
const sectionControllers = new Map<ToolDetailSection, AbortController>()
function cancelSectionRequests() {
  for (const controller of sectionControllers.values()) controller.abort()
  sectionControllers.clear()
}
const sectionValues = ref<Partial<Record<ToolDetailSection, JsonValue>>>({})
const loadingSections = ref<Set<ToolDetailSection>>(new Set())
const sectionErrors = ref<Partial<Record<ToolDetailSection, string>>>({})
const loadedPartKey = ref('')
const toolDetailSections: ToolDetailSection[] = ['input', 'output', 'metadata', 'presentation']

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
  return presentationExpanded.value
}

function setSectionExpanded(section: ToolDetailSection, expanded: boolean) {
  if (section === 'metadata') metadataExpanded.value = expanded
  else if (section === 'input') inputExpanded.value = expanded
  else if (section === 'output') outputExpanded.value = expanded
  else presentationExpanded.value = expanded
}

type SectionLoadOptions = { force?: boolean; attempt?: number }

async function loadSection(section: ToolDetailSection, options: SectionLoadOptions = {}) {
  // Presentation is part of every transcript snapshot. The other sections
  // are deliberately fetched only after their disclosure row is opened, and a
  // forced refresh keeps the rendered snapshot on screen until the new one
  // arrives.
  if (section === 'presentation' || sectionLoading(section)) return
  if (!options.force && !sectionError(section) && sectionLoaded(section)) return
  const sessionId = String(props.sessionId || '').trim()
  const partId = String(props.part.id || '').trim()
  if (!sessionId || !partId) return
  const attempt = options.attempt ?? 0
  const requestGeneration = partGeneration
  const controller = new AbortController()
  sectionControllers.set(section, controller)
  const timeout = setTimeout(() => controller.abort(), 30_000)

  loadingSections.value = new Set([...loadingSections.value, section])
  sectionErrors.value = { ...sectionErrors.value, [section]: '' }
  try {
    const resource = await getToolPartDetail(sessionId, partId, section, controller.signal)
    if (partGeneration !== requestGeneration) return
    if (resource.part_id !== Number(partId) || resource.section !== section) {
      throw new Error('The server returned a mismatched tool detail section')
    }
    const revision = props.part.source.revision ?? 0
    const updatedAt = props.part.source.updatedAt ?? 0
    if (resource.revision < revision || (resource.revision === revision && resource.updated_at_ms < updatedAt)) {
      // The tool moved on while this section was loading. While it is still
      // streaming, retry quietly and keep the rendered snapshot instead of
      // flashing an error into the part the reader has expanded.
      if (!status.value.terminal && attempt < MAX_STALE_SECTION_RETRIES) {
        scheduleStaleSectionRetry(section, attempt + 1, requestGeneration)
        return
      }
      throw new Error('This tool changed while loading its details. Retry this section.')
    }
    sectionValues.value = { ...sectionValues.value, [section]: resource.value }
  } catch (error) {
    if (partGeneration !== requestGeneration) return
    sectionErrors.value = {
      ...sectionErrors.value,
      [section]: error instanceof Error ? error.message : 'Unable to load this section',
    }
  } finally {
    clearTimeout(timeout)
    if (sectionControllers.get(section) === controller) sectionControllers.delete(section)
    if (partGeneration === requestGeneration) {
      const next = new Set(loadingSections.value)
      next.delete(section)
      loadingSections.value = next
    }
  }
}

async function toggleSection(section: ToolDetailSection) {
  emit('select')
  const expanded = !sectionExpanded(section)
  setSectionExpanded(section, expanded)
  if (expanded) await loadSection(section)
}

function toggleDetails() {
  emit('select')
  detailsExpanded.value = !detailsExpanded.value
}

function loadVisibleSections() {
  if (props.expanded && detailsExpanded.value) {
    for (const section of toolDetailSections) {
      // Reopening a child refreshes it, but the rendered snapshot stays on
      // screen until the new one arrives instead of blanking the section.
      if (sectionExpanded(section)) void loadSection(section, { force: sectionLoaded(section) })
    }
  }
}

function resetSectionState() {
  detailsExpanded.value = false
  metadataExpanded.value = false
  inputExpanded.value = false
  outputExpanded.value = false
  presentationExpanded.value = false
}

watch(
  () => `${props.sessionId || ''}:${props.part.id || ''}`,
  (key) => {
    partGeneration += 1
    cancelSectionRequests()
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

watch([() => props.expanded, detailsExpanded], loadVisibleSections)

const STALE_SECTION_RETRY_MS = 400
const MAX_STALE_SECTION_RETRIES = 2

function scheduleStaleSectionRetry(section: ToolDetailSection, attempt: number, generation: number) {
  window.setTimeout(() => {
    if (partGeneration !== generation || !sectionExpanded(section)) return
    void loadSection(section, { force: true, attempt })
  }, STALE_SECTION_RETRY_MS)
}

// A running tool bumps its revision on every streamed update. Refetching on
// each bump dropped the rendered sections and reloaded them over and over
// while the reply kept streaming, which is the flashing an expanded part must
// not do. Keep the rendered snapshot while the part is live and refresh the
// expanded sections once it settles, so a stale running snapshot still cannot
// overwrite the completed result.
watch(
  () => [props.part.status, props.part.source.revision, props.part.source.updatedAt] as const,
  (next, previous) => {
    if (!previous || !status.value.terminal) return
    const changed = next[0] !== previous[0] || next[1] !== previous[1] || next[2] !== previous[2]
    if (!changed) return
    partGeneration += 1
    cancelSectionRequests()
    loadingSections.value = new Set()
    if (!props.expanded || !detailsExpanded.value) return
    for (const section of toolDetailSections) {
      if (sectionExpanded(section)) void loadSection(section, { force: true })
    }
  },
)
onBeforeUnmount(() => {
  partGeneration += 1
  cancelSectionRequests()
})

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

      <div class="space-y-3 py-1" data-tool-presentation>
        <MarkdownRenderer
          v-if="operation.commandMarkdown"
          :content="operation.commandMarkdown"
          mode="markdown"
          :stream="false"
        />
        <MarkdownRenderer
          v-if="
            operation.summary &&
            !operation.presentationBlocks.length &&
            !operation.userInputs.length &&
            !operation.permissions.length &&
            !operation.error
          "
          :content="operation.summary"
          mode="markdown"
          :stream="false"
        />
        <AgenaOperationBlock
          v-for="(block, index) in operation.presentationBlocks"
          :key="String(block.id || `${block.type || block.kind || 'block'}:${index}`)"
          :block="block"
        />
      </div>

      <button
        type="button"
        class="flex min-h-8 items-center gap-2 rounded-md px-1 py-1 text-xs text-muted-foreground outline-none hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-ring/50"
        :aria-expanded="detailsExpanded"
        data-tool-details-toggle
        @click="toggleDetails"
      >
        <span class="w-3 text-center font-mono" aria-hidden="true">{{ detailsExpanded ? '▾' : '▸' }}</span>
        {{ t('chat.toolDetails.label') }}
      </button>
      <div v-if="detailsExpanded" class="ml-2 border-l border-border/40 pl-3" data-tool-details>
        <section v-for="section in toolDetailSections" :key="section" class="py-1">
          <button
            type="button"
            class="flex min-h-8 items-center gap-2 rounded-md px-1 py-1 text-xs font-semibold text-muted-foreground outline-none hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-ring/50"
            :aria-expanded="sectionExpanded(section)"
            :data-tool-detail-section="section"
            @click="toggleSection(section)"
          >
            <span class="w-3 text-center font-mono text-muted-foreground" aria-hidden="true">{{
              sectionExpanded(section) ? '▾' : '▸'
            }}</span>
            {{ t(`chat.toolDetails.${section}`) }}
            <span v-if="sectionLoading(section)" class="font-normal text-muted-foreground">{{
              t('common.loading')
            }}</span>
          </button>

          <div v-if="sectionExpanded(section)" class="min-w-0 pl-5 pt-1">
            <div v-if="sectionError(section)" role="alert" class="py-1 text-xs text-rose-700 dark:text-rose-300">
              {{ sectionError(section) }}
              <button type="button" class="ml-2 underline" @click="loadSection(section)">
                {{ t('common.retry') }}
              </button>
            </div>
            <template v-else-if="sectionLoading(section)" />
            <template v-else-if="section === 'metadata'">
              <MarkdownRenderer
                :content="structuredValueMarkdown(operation.metadata)"
                mode="markdown"
                :stream="false"
              />
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
              <CodeBlock :code="prettyJson(operation.rawOutput)" lang="json" compact />
            </template>
            <CodeBlock v-else :code="prettyJson(part.source.agenaPresentation)" lang="json" compact />
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
