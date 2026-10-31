<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiGlobalLine } from '@remixicon/vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import AgenaDiffBlock from '@/components/chat/AgenaDiffBlock.vue'
import AgenaContentOutput from '@/components/chat/AgenaContentOutput.vue'
import AgenaAttachmentPreview from '@/components/chat/AgenaAttachmentPreview.vue'
import AgenaResourceLink from '@/components/chat/AgenaResourceLink.vue'
import { useChatStore } from '@/stores/chat'
import { workspaceAttachmentPath } from '@/lib/attachmentLinks'
import { openResource } from '@/lib/authenticatedResources'
import { useDirectoryStore } from '@/stores/directory'
import { useUiStore } from '@/stores/ui'
import type { ContentFormat, ContentRef } from '@/lib/content'
import {
  jsonArray,
  fencedCodeBlock,
  jsonRecord,
  prettyJson,
  stringValue,
  structuredValueMarkdown,
  operationAttachmentPresentation,
  type JsonRecord,
} from '@/pages/chat/transcriptPartPresentation'
import type { JsonValue } from '@/types/json'

const props = defineProps<{ block: JsonRecord; sessionId?: string | null }>()
const chat = useChatStore()
const directory = useDirectoryStore()
const ui = useUiStore()
const mediaAttachment = computed(() => operationAttachmentPresentation(props.block))
const workspaceRoot = computed(() => chat.getSessionDirectory(props.sessionId) || directory.currentDirectory || '')
const mediaOpenError = ref('')

async function openMedia(path: string, url: string) {
  mediaOpenError.value = ''
  const attachment = mediaAttachment.value
  if ((attachment?.kind === 'image' || attachment?.mime.startsWith('image/')) && url) {
    ui.openImageViewer([{ src: url, alt: attachment.label, title: attachment.label, key: attachment.key }])
  } else {
    const dockPath = workspaceAttachmentPath(path, workspaceRoot.value, directory.currentDirectory || '')
    if (dockPath) ui.requestWorkspaceDockFile(dockPath, 'open')
    else if (url) {
      try {
        await openResource(url, attachment?.label)
      } catch (error) {
        mediaOpenError.value = error instanceof Error ? error.message : String(error)
      }
    }
  }
}

const contentResource = computed(() => jsonRecord(props.block.resource) as unknown as ContentRef)
const contentFormat = computed(
  () =>
    (['markdown', 'code', 'diff', 'json'].includes(stringValue(props.block.format))
      ? stringValue(props.block.format)
      : 'plain') as ContentFormat,
)

const kind = computed(() => stringValue(props.block.type) || 'unknown')
const bodyText = computed(() => stringValue(props.block.text))
const command = computed(() => stringValue(props.block.command))
// A command is Markdown code, not prose: render it through the shared Markdown
// fence so the complete command stays readable and copyable.
const commandMarkdown = computed(() => (command.value.trim() ? fencedCodeBlock(command.value, 'sh') : ''))
const cwd = computed(() => stringValue(props.block.cwd))
const stdout = computed(() => stringValue(props.block.stdout))
const stderr = computed(() => stringValue(props.block.stderr))
const exitCode = computed(() => (typeof props.block.exit_code === 'number' ? props.block.exit_code : null))

const tableColumns = computed(() => jsonArray(props.block.columns).map(stringValue))
const tableRows = computed(() => jsonArray(props.block.rows).filter(Array.isArray))

function formatCell(value: JsonValue | undefined): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  if (Array.isArray(value)) return value.map(formatCell).filter(Boolean).join(' · ')
  return Object.entries(value)
    .map(([key, item]) => `${key}: ${formatCell(item)}`)
    .join(' · ')
}

const searchResults = computed(() =>
  jsonArray(props.block.items).map((raw, index) => {
    const item = jsonRecord(raw)
    const uri = stringValue(item.url)
    return {
      key: uri || `${stringValue(item.title)}:${index}`,
      title: stringValue(item.title) || uri || `Result ${index + 1}`,
      uri,
      snippet: stringValue(item.snippet),
    }
  }),
)

const fileChanges = computed(() =>
  jsonArray(props.block.changes).map((raw, index) => {
    const item = jsonRecord(raw)
    return {
      key: stringValue(item.path) || String(index),
      path: stringValue(item.path) || `file-${index + 1}`,
      status: stringValue(item.kind),
    }
  }),
)

const progressPercent = computed(() =>
  typeof props.block.total === 'number' && props.block.total > 0 && typeof props.block.completed === 'number'
    ? Math.max(0, Math.min(100, Math.floor((props.block.completed * 100) / props.block.total)))
    : null,
)
const progressCount = computed(() =>
  typeof props.block.completed === 'number'
    ? `${props.block.completed}${typeof props.block.total === 'number' ? `/${props.block.total}` : ''}${stringValue(props.block.unit) ? ` ${stringValue(props.block.unit)}` : ''}`
    : '',
)
</script>

<template>
  <div class="min-w-0 text-[13px] leading-snug">
    <AgenaContentOutput
      v-if="kind === 'content' && sessionId"
      :key="contentResource.resource_id"
      :resource="contentResource"
      :session-id="sessionId"
      :format="contentFormat"
    />
    <MarkdownRenderer v-else-if="kind === 'markdown'" :content="bodyText" mode="markdown" :stream="false" />

    <pre
      v-else-if="kind === 'text' || kind === 'log'"
      class="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs leading-snug text-foreground/90"
      >{{ bodyText }}</pre
    >

    <div v-else-if="kind === 'command'" class="space-y-1">
      <div v-if="cwd" class="font-mono text-[10px] text-muted-foreground">{{ cwd }}</div>
      <MarkdownRenderer v-if="commandMarkdown" :content="commandMarkdown" mode="markdown" :stream="false" />
      <CodeBlock v-if="stdout" :code="stdout" lang="text" compact />
      <CodeBlock v-if="stderr" :code="stderr" lang="text" compact />
      <div v-if="exitCode !== null" class="font-mono text-[11px] text-muted-foreground">exit {{ exitCode }}</div>
    </div>

    <MarkdownRenderer
      v-else-if="kind === 'json'"
      :content="structuredValueMarkdown(block.value ?? null)"
      mode="markdown"
      :stream="false"
    />

    <AgenaDiffBlock v-else-if="kind === 'diff'" :diff="stringValue(block.diff)" />

    <div v-else-if="kind === 'table'" class="overflow-x-auto border-y border-border/60">
      <table class="w-full border-collapse text-left text-xs">
        <thead class="text-muted-foreground">
          <tr>
            <th v-for="column in tableColumns" :key="column" class="border-b border-border/60 px-2 py-1 font-medium">
              {{ column }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, rowIndex) in tableRows" :key="rowIndex" class="border-b border-border/35 last:border-b-0">
            <td
              v-for="(column, columnIndex) in tableColumns"
              :key="column"
              class="min-w-24 max-w-xl break-words px-2 py-1 align-top whitespace-pre-wrap"
            >
              {{ formatCell(row[columnIndex]) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-else-if="kind === 'search_results'" class="interaction-width space-y-2">
      <div v-for="result in searchResults" :key="result.key" class="space-y-1">
        <AgenaResourceLink :href="result.uri" :label="result.title" :description="result.uri" :icon="RiGlobalLine" />
        <p v-if="result.snippet" class="px-1 text-xs leading-relaxed text-muted-foreground">{{ result.snippet }}</p>
      </div>
    </div>

    <div
      v-else-if="kind === 'file_changes'"
      class="divide-y divide-border/40 border-y border-border/60 font-mono text-xs"
    >
      <div v-for="change in fileChanges" :key="change.key" class="flex min-w-0 items-center gap-3 py-1">
        <span class="min-w-0 flex-1 truncate">{{ change.path }}</span>
        <span v-if="change.status" class="text-muted-foreground">{{ change.status }}</span>
      </div>
    </div>

    <div v-else-if="mediaAttachment" class="space-y-1">
      <AgenaAttachmentPreview
        :attachment="mediaAttachment"
        :session-id="sessionId"
        :workspace-root="workspaceRoot"
        @open="openMedia"
      />
      <p v-if="mediaOpenError" class="text-xs text-destructive" role="alert">{{ mediaOpenError }}</p>
    </div>

    <div v-else-if="kind === 'progress'" class="space-y-1.5 border-y border-border/50 py-1">
      <div class="flex items-center justify-between gap-3 text-xs">
        <span class="min-w-0 break-words">{{ stringValue(block.phase) }}</span>
        <span class="shrink-0 font-mono text-[10px] text-muted-foreground">{{ progressCount }}</span>
        <span v-if="progressPercent !== null" class="shrink-0 font-mono text-[10px] text-muted-foreground">
          {{ progressPercent }}%
        </span>
      </div>
      <div
        v-if="progressPercent !== null"
        class="h-1 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        :aria-valuenow="progressPercent"
        :aria-valuemin="0"
        :aria-valuemax="100"
        :aria-label="stringValue(block.phase)"
      >
        <div class="h-full bg-primary" :style="{ width: `${progressPercent}%` }" />
      </div>
    </div>

    <MarkdownRenderer
      v-else-if="kind === 'custom' && Object.keys(jsonRecord(block.presentation)).length"
      :content="structuredValueMarkdown(block.presentation ?? {})"
      mode="markdown"
      :stream="false"
    />

    <CodeBlock
      v-else
      :code="prettyJson(kind === 'custom' ? (block.schema ?? block) : (block.value ?? block.presentation ?? block))"
      lang="json"
      compact
    />
  </div>
</template>
