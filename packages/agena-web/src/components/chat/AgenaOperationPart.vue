<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowRightSLine } from '@remixicon/vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import AgenaInteractionPart from '@/components/chat/AgenaInteractionPart.vue'
import AgenaOperationBlock from '@/components/chat/AgenaOperationBlock.vue'
import ActivityLogView from '@/components/chat/ActivityLogView.vue'
import PartLoadingIndicator from '@/components/chat/PartLoadingIndicator.vue'
import { useChatStore } from '@/stores/chat'
import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'
import {
  operationPresentation,
  partStatusPresentation,
  prettyJson,
  structuredValueMarkdown,
} from '@/pages/chat/transcriptPartPresentation'
import { getToolPartDetail, type ToolDetailSection } from '@/stores/chat/api'
import { isDocumentVisible } from '@/lib/backgroundReads'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import {
  canReuseResource,
  captureResourceObservation,
  checkResourceVersions,
  subscribeResource,
} from '@/lib/resourceSync'
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
const pane = useWorkspacePaneContext()
const isVisible = () => isDocumentVisible() && (!pane || pane.isVisible.value)
const detailsExpanded = ref(false)
const metadataExpanded = ref(false)
const inputExpanded = ref(false)
const outputExpanded = ref(false)
const presentationExpanded = ref(false)
let partGeneration = 0
let liveRefreshTimer: ReturnType<typeof setTimeout> | undefined
let liveRefreshScheduledAt = Infinity
let lastLiveRefreshAt = -Infinity
let liveRefreshAllowedAt = -Infinity
const sectionFailures = new Map<ToolDetailSection, number>()
const sectionAllowedAt = new Map<ToolDetailSection, number>()
const sectionControllers = new Map<ToolDetailSection, AbortController>()
const queuedSections = reactive(new Set<ToolDetailSection>())
const sectionSubscriptions = new Map<string, () => void>()
const sectionObservations = new Map<ToolDetailSection, ReturnType<typeof captureResourceObservation>>()
const sectionRevisions = new Map<ToolDetailSection, number>()
let loadedOutputState: string | undefined
function cancelSectionRequests() {
  clearTimeout(liveRefreshTimer)
  liveRefreshTimer = undefined
  liveRefreshScheduledAt = Infinity
  for (const controller of sectionControllers.values()) controller.abort()
  sectionControllers.clear()
  queuedSections.clear()
  loadingSections.value = new Set()
}
const sectionValues = shallowRef<Partial<Record<ToolDetailSection, JsonValue>>>({})
const loadingSections = ref<Set<ToolDetailSection>>(new Set())
const sectionErrors = ref<Partial<Record<ToolDetailSection, string>>>({})
const loadedPartKey = ref('')
const toolDetailSections: ToolDetailSection[] = ['input', 'output', 'metadata', 'presentation']

const operation = computed(() => operationPresentation(props.part, sectionValues.value, props.expanded))
const hasContentOutput = computed(() => operation.value.presentationBlocks.some((block) => block.type === 'content'))
const status = computed(() => partStatusPresentation(props.part.status))
const chat = useChatStore()
const linkedActivity = computed(() =>
  props.sessionId
    ? (chat
        .sessionBackgroundActivities(props.sessionId)
        .find((activity) => activity.source_part_id === Number(props.part.id)) ?? null)
    : null,
)

function sectionLoaded(section: ToolDetailSection): boolean {
  return sectionHasValue(section) && sectionRevisions.get(section) === (props.part.source.revision ?? 0)
}

function sectionHasValue(section: ToolDetailSection): boolean {
  return Object.prototype.hasOwnProperty.call(sectionValues.value, section)
}

function sectionLoading(section: ToolDetailSection): boolean {
  return loadingSections.value.has(section)
}

function sectionError(section: ToolDetailSection): string {
  return sectionErrors.value[section] || ''
}

function sectionPending(section: ToolDetailSection): boolean {
  return sectionLoading(section) || (queuedSections.has(section) && !sectionError(section))
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

type SectionLoadOptions = { force?: boolean }

async function loadSection(section: ToolDetailSection, options: SectionLoadOptions = {}) {
  // Presentation is part of every transcript snapshot. The other sections
  // are deliberately fetched only after their disclosure row is opened, and a
  // forced refresh keeps the rendered snapshot on screen until the new one
  // arrives.
  if (section === 'presentation' || !isVisible()) return
  if (sectionLoading(section)) {
    if (options.force) queuedSections.add(section)
    return
  }
  if (!options.force && !sectionError(section) && sectionLoaded(section)) return
  const sessionId = String(props.sessionId || '').trim()
  const partId = String(props.part.id || '').trim()
  if (!sessionId || !partId) return
  if (Date.now() < Math.max(liveRefreshAllowedAt, sectionAllowedAt.get(section) ?? -Infinity)) {
    scheduleLiveSectionRefresh([section])
    return
  }
  const requestGeneration = partGeneration
  const hadSectionError = Boolean(sectionError(section))
  const controller = new AbortController()
  queuedSections.delete(section)
  sectionControllers.set(section, controller)
  const timeout = setTimeout(() => controller.abort(), 30_000)

  loadingSections.value = new Set([...loadingSections.value, section])
  sectionErrors.value = { ...sectionErrors.value, [section]: '' }
  try {
    const key = `part:${partId}:${section}`
    const sourceState = props.part.source.partState ?? props.part.status
    // A terminal transcript can arrive before its section's revision hint.
    // Validate output directly until its actual body reflects that terminal
    // state, without refreshing the independent input/metadata sections.
    const needsTerminalOutput =
      section === 'output' && status.value.terminal && sectionLoaded(section) && loadedOutputState !== sourceState
    const observed = sectionObservations.get(section)
    if (
      !needsTerminalOutput &&
      sectionLoaded(section) &&
      observed?.token &&
      observed.scope === captureResourceObservation(key).scope &&
      !hadSectionError
    ) {
      await checkResourceVersions([key], controller.signal)
      if (canReuseResource(key, observed.token)) return
    }
    const resource = await getToolPartDetail(sessionId, partId, section, controller.signal, needsTerminalOutput)
    if (
      partGeneration !== requestGeneration ||
      controller.signal.aborted ||
      sectionControllers.get(section) !== controller
    )
      return
    if (resource.part_id !== Number(partId) || resource.section !== section) {
      throw new Error('The server returned a mismatched tool detail section')
    }
    if (resource.revision < (sectionRevisions.get(section) ?? -1)) return
    const revision = props.part.source.revision ?? 0
    const updatedAt = props.part.source.updatedAt ?? 0
    const olderEnvelope =
      resource.revision < revision || (resource.revision === revision && resource.updated_at_ms < updatedAt)
    if (olderEnvelope && section !== 'output') scheduleLiveSectionRefresh([section])
    const staleOutput = resource.part_state
      ? resource.part_state !== props.part.source.partState && (status.value.terminal || olderEnvelope)
      : olderEnvelope
    if (section === 'output' && staleOutput) {
      // The tool moved on while this section was loading. While it is still
      // streaming, retry quietly and keep the rendered snapshot instead of
      // flashing an error into the part the reader has expanded.
      if (status.value.terminal) throw new Error('This tool changed while loading its details. Retry this section.')
      // Live snapshots are best effort. Keep this useful intermediate value
      // on screen and schedule catch-up, rather than starving a section while
      // its source advances faster than HTTP can round-trip.
      scheduleLiveSectionRefresh([section])
    }
    if (!sectionLoaded(section) || sectionValues.value[section] !== resource.value)
      sectionValues.value = { ...sectionValues.value, [section]: resource.value }
    sectionRevisions.set(section, resource.revision)
    if (resource.observation) sectionObservations.set(section, resource.observation)
    else sectionObservations.delete(section)
    if (section === 'output') loadedOutputState = resource.part_state ?? sourceState
    sectionFailures.delete(section)
    sectionAllowedAt.set(section, Date.now() + 750)
    liveRefreshAllowedAt = Math.max(liveRefreshAllowedAt, Date.now() + 750)
  } catch (error) {
    if (partGeneration !== requestGeneration || sectionControllers.get(section) !== controller) return
    sectionErrors.value = {
      ...sectionErrors.value,
      [section]: error instanceof Error ? error.message : 'Unable to load this section',
    }
    const failures = Math.min((sectionFailures.get(section) ?? 0) + 1, 6)
    sectionFailures.set(section, failures)
    sectionAllowedAt.set(section, Date.now() + Math.min(60_000, 5000 * 2 ** (failures - 1)))
  } finally {
    clearTimeout(timeout)
    const ownsRequest = sectionControllers.get(section) === controller
    if (ownsRequest) sectionControllers.delete(section)
    if (partGeneration === requestGeneration && ownsRequest) {
      const next = new Set(loadingSections.value)
      next.delete(section)
      loadingSections.value = next
      if (sectionError(section)) queuedSections.add(section)
      if (
        queuedSections.has(section) &&
        props.expanded &&
        detailsExpanded.value &&
        sectionExpanded(section) &&
        isVisible()
      )
        scheduleLiveSectionRefresh([])
    }
  }
}

async function toggleSection(section: ToolDetailSection) {
  emit('select')
  const expanded = !sectionExpanded(section)
  setSectionExpanded(section, expanded)
  if (expanded) await loadSection(section, { force: sectionLoaded(section) })
}

function toggleDetails() {
  emit('select')
  detailsExpanded.value = !detailsExpanded.value
}

function loadVisibleSections() {
  if (isVisible() && props.expanded && detailsExpanded.value) {
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
      sectionObservations.clear()
      sectionRevisions.clear()
      loadedOutputState = undefined
    }
    loadedPartKey.value = key
    lastLiveRefreshAt = liveRefreshAllowedAt = -Infinity
    sectionFailures.clear()
    sectionAllowedAt.clear()
    resetSectionState()
  },
  { immediate: true },
)

watch(
  () => [props.part.source.agenaSections, props.part.source.agenaContent, props.part.source.agenaPresentation] as const,
  () => {
    const content = props.part.source.agenaContent
    const fields = content && typeof content === 'object' && !Array.isArray(content) ? content : {}
    for (const loaded of props.part.source.agenaSections || []) {
      if ((sectionRevisions.get(loaded.section) ?? -1) > loaded.revision) continue
      const value =
        loaded.section === 'presentation'
          ? (props.part.source.agenaPresentation ?? null)
          : (fields[loaded.section] ?? null)
      sectionValues.value = { ...sectionValues.value, [loaded.section]: value }
      sectionRevisions.set(loaded.section, loaded.revision)
    }
  },
  { immediate: true },
)

// These disclosures are opened explicitly by the reader. Idle transitions
// must keep their state and the last rendered value, just like part details.

watch([() => props.expanded, detailsExpanded], loadVisibleSections)
watch(
  () =>
    [
      props.expanded && detailsExpanded.value,
      metadataExpanded.value,
      inputExpanded.value,
      outputExpanded.value,
    ] as const,
  ([open]) => {
    if (!open) {
      cancelSectionRequests()
      return
    }
    for (const [section, controller] of sectionControllers) {
      if (sectionExpanded(section)) continue
      controller.abort()
      sectionControllers.delete(section)
      queuedSections.delete(section)
      const next = new Set(loadingSections.value)
      next.delete(section)
      loadingSections.value = next
    }
    for (const section of queuedSections) if (!sectionExpanded(section)) queuedSections.delete(section)
    if (!queuedSections.size) {
      clearTimeout(liveRefreshTimer)
      liveRefreshTimer = undefined
      liveRefreshScheduledAt = Infinity
    }
  },
  { flush: 'sync' },
)
function syncSectionSubscriptions() {
  const open = props.expanded && detailsExpanded.value && isVisible()
  const keys = new Map<string, ToolDetailSection>(
    open
      ? toolDetailSections
          .filter((section) => section !== 'presentation' && sectionExpanded(section))
          .map((section) => [`part:${props.part.id}:${section}`, section] as const)
      : [],
  )
  for (const [key, release] of sectionSubscriptions)
    if (!keys.has(key)) {
      release()
      sectionSubscriptions.delete(key)
    }
  for (const [key, section] of keys)
    if (!sectionSubscriptions.has(key))
      sectionSubscriptions.set(
        key,
        subscribeResource(key, () => scheduleLiveSectionRefresh([section])),
      )
}
watch(
  () => [
    props.part.id,
    props.expanded,
    detailsExpanded.value,
    metadataExpanded.value,
    inputExpanded.value,
    outputExpanded.value,
  ],
  syncSectionSubscriptions,
  { immediate: true, flush: 'sync' },
)

function scheduleLiveSectionRefresh(sections = toolDetailSections) {
  if (!isVisible() || !props.expanded || !detailsExpanded.value) return
  for (const section of sections)
    if (section !== 'presentation' && sectionExpanded(section)) queuedSections.add(section)
  const ready = [...queuedSections].filter((section) => sectionExpanded(section) && !sectionLoading(section))
  if (!ready.length) return
  const now = Date.now()
  const earliest = Math.min(...ready.map((section) => sectionAllowedAt.get(section) ?? -Infinity))
  const delay = Math.max(750, 1000 - (now - lastLiveRefreshAt), liveRefreshAllowedAt - now, earliest - now)
  const due = now + delay
  if (liveRefreshTimer && liveRefreshScheduledAt <= due) return
  clearTimeout(liveRefreshTimer)
  liveRefreshScheduledAt = due
  liveRefreshTimer = setTimeout(() => {
    liveRefreshTimer = undefined
    liveRefreshScheduledAt = Infinity
    if (!isVisible() || !props.expanded || !detailsExpanded.value) return
    // A response/error arriving after scheduling can extend the cooldown.
    if (Date.now() < liveRefreshAllowedAt) {
      scheduleLiveSectionRefresh([])
      return
    }
    const now = Date.now()
    for (const section of [...queuedSections]) {
      if (!sectionExpanded(section)) {
        queuedSections.delete(section)
        continue
      }
      if (sectionLoading(section) || now < (sectionAllowedAt.get(section) ?? -Infinity)) continue
      queuedSections.delete(section)
      lastLiveRefreshAt = now
      void loadSection(section, { force: true })
    }
    scheduleLiveSectionRefresh([])
  }, delay)
}

// Coalesce live updates without clearing the rendered value. Terminal
// transitions cancel old requests, so no running value can replace a result.
/**
 * Refresh the expanded sections of a settled part. Reloading them is display
 * work, so a hidden tab defers it until the reader is looking again.
 */
function refreshSettledSections(sections: ToolDetailSection[] = ['output']) {
  if (!isVisible() || !props.expanded || !detailsExpanded.value) return
  scheduleLiveSectionRefresh(sections)
}

watch(
  () => [props.part.status, props.part.source.revision, props.part.source.updatedAt] as const,
  (next, previous) => {
    if (!previous) return
    const changed = next[0] !== previous[0] || next[1] !== previous[1] || next[2] !== previous[2]
    if (!changed) return
    // Input/metadata follow their own clocks. Status and live output only
    // affect output; a part update must not reload every open disclosure.
    if (!status.value.terminal || next[0] === previous[0]) {
      scheduleLiveSectionRefresh(['output'])
      return
    }
    const interrupted = [...new Set([...sectionControllers.keys(), ...queuedSections, 'output' as const])]
    partGeneration += 1
    cancelSectionRequests()
    loadingSections.value = new Set()
    refreshSettledSections(interrupted)
  },
)
function handleVisibilityChange() {
  syncSectionSubscriptions()
  if (!isVisible()) {
    partGeneration += 1
    cancelSectionRequests()
    loadingSections.value = new Set()
  } else loadVisibleSections()
  syncSectionSubscriptions()
}
document.addEventListener('visibilitychange', handleVisibilityChange)
if (pane) watch(pane.isVisible, handleVisibilityChange, { flush: 'sync' })
onBeforeUnmount(() => {
  partGeneration += 1
  cancelSectionRequests()
  for (const release of sectionSubscriptions.values()) release()
  document.removeEventListener('visibilitychange', handleVisibilityChange)
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
      class="group/headline flex min-h-7 w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-0.5 text-left outline-none hover:bg-muted/35 focus-visible:ring-1 focus-visible:ring-ring/50"
      :aria-expanded="expanded"
      data-transcript-vim-toggle="true"
      @click="toggleOuter"
      @focus="$emit('select')"
    >
      <RiArrowRightSLine
        class="h-5 w-5 shrink-0 text-muted-foreground"
        :class="expanded ? 'rotate-90' : ''"
        aria-hidden="true"
        data-part-disclosure-icon
      />
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

    <div v-if="expanded" class="transcript-content bg-muted/[0.08]">
      <div v-if="operation.userInputs.length" class="space-y-2 py-0.5 text-sm">
        <AgenaInteractionPart
          v-for="interaction in operation.userInputs"
          :key="interaction.requestId || interaction.title"
          :interaction="interaction"
          :session-id="sessionId"
        />
      </div>

      <section v-if="operation.permissions.length" class="space-y-2 py-0.5 text-sm">
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
          class="mt-1 whitespace-pre-wrap break-words font-mono text-xs leading-snug text-rose-700 dark:text-rose-300"
          >{{ operation.error }}</pre
        >
      </section>

      <div
        v-if="linkedActivity || operation.commandMarkdown || operation.summary || operation.presentationBlocks.length"
        class="space-y-1 py-0.5"
        data-tool-presentation
      >
        <ActivityLogView
          v-if="linkedActivity && sessionId && !hasContentOutput"
          :session-id="sessionId"
          :activity="linkedActivity"
        />
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
          :session-id="sessionId"
        />
      </div>

      <button
        type="button"
        class="flex min-h-6 items-center gap-2 rounded-md px-1 py-0.5 text-xs text-muted-foreground outline-none hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-ring/50"
        :aria-expanded="detailsExpanded"
        data-tool-details-toggle
        @click="toggleDetails"
      >
        <RiArrowRightSLine class="h-4 w-4 shrink-0" :class="detailsExpanded ? 'rotate-90' : ''" aria-hidden="true" />
        {{ t('chat.toolDetails.label') }}
      </button>
      <div v-if="detailsExpanded" class="ml-2 border-l border-border/40 pl-2" data-tool-details>
        <section v-for="section in toolDetailSections" :key="section" class="py-0.5">
          <button
            type="button"
            class="flex min-h-6 items-center gap-2 rounded-md px-1 py-0.5 text-xs font-semibold text-muted-foreground outline-none hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-ring/50"
            :aria-expanded="sectionExpanded(section)"
            :aria-busy="sectionPending(section)"
            :data-tool-detail-section="section"
            @click="toggleSection(section)"
          >
            <RiArrowRightSLine
              class="h-4 w-4 shrink-0"
              :class="sectionExpanded(section) ? 'rotate-90' : ''"
              aria-hidden="true"
            />
            {{ t(`chat.toolDetails.${section}`) }}
            <PartLoadingIndicator v-if="sectionPending(section)" />
          </button>

          <div v-if="sectionExpanded(section)" class="min-w-0 pl-3 pt-1" :aria-busy="sectionPending(section)">
            <div v-if="sectionError(section)" role="alert" class="py-1 text-xs text-rose-700 dark:text-rose-300">
              {{ sectionError(section) }}
              <button type="button" class="ml-2 underline" @click="loadSection(section)">
                {{ t('common.retry') }}
              </button>
            </div>
            <div
              v-if="sectionPending(section) && !sectionHasValue(section)"
              class="space-y-2 py-2"
              aria-hidden="true"
              data-part-loading-placeholder
            >
              <div class="h-3 w-3/4 animate-pulse rounded bg-muted" />
              <div class="h-3 w-1/2 animate-pulse rounded bg-muted" />
            </div>
            <template v-else-if="sectionError(section) && !sectionHasValue(section)" />
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
