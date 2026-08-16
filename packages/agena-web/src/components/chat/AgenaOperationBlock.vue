<script setup lang="ts">
import { computed } from 'vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import {
  jsonArray,
  jsonRecord,
  prettyJson,
  stringValue,
  type JsonRecord,
} from '@/pages/chat/transcriptPartPresentation'
import type { JsonValue } from '@/types/json'

const props = defineProps<{ block: JsonRecord }>()

const kind = computed(() => stringValue(props.block.type) || stringValue(props.block.kind) || 'unknown')
const bodyText = computed(() => {
  return (
    stringValue(props.block.text) ||
    stringValue(props.block.markdown) ||
    stringValue(props.block.stdout) ||
    stringValue(props.block.content)
  )
})
const command = computed(() => stringValue(props.block.command))
const cwd = computed(() => stringValue(props.block.cwd))
const stdout = computed(() => stringValue(props.block.stdout))
const stderr = computed(() => stringValue(props.block.stderr))
const exitCode = computed(() => (typeof props.block.exit_code === 'number' ? props.block.exit_code : null))

const tableColumns = computed(() =>
  jsonArray(props.block.columns).map((raw, index) => {
    if (typeof raw === 'string') return { key: raw, label: raw }
    const column = jsonRecord(raw)
    const key = stringValue(column.key) || String(index)
    return { key, label: stringValue(column.label) || key }
  }),
)
const tableRows = computed(() => jsonArray(props.block.rows).map((row) => row))

function tableCell(row: JsonValue, key: string, index: number): string {
  if (Array.isArray(row)) return formatCell(row[index])
  const record = jsonRecord(row)
  return formatCell(record[key])
}

function formatCell(value: JsonValue | undefined): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}

const searchResults = computed(() => {
  const source = jsonArray(props.block.results).length ? jsonArray(props.block.results) : jsonArray(props.block.items)
  return source.map((raw, index) => {
    const item = jsonRecord(raw)
    const uri = stringValue(item.uri) || stringValue(item.url)
    return {
      key: uri || `${stringValue(item.title)}:${index}`,
      title: stringValue(item.title) || uri || `Result ${index + 1}`,
      uri,
      snippet: stringValue(item.snippet) || stringValue(item.description),
    }
  })
})

const fileChanges = computed(() => {
  const source = jsonArray(props.block.changes).length ? jsonArray(props.block.changes) : jsonArray(props.block.files)
  return source.map((raw, index) => {
    const item = jsonRecord(raw)
    return {
      key: stringValue(item.path) || stringValue(item.filename) || String(index),
      path: stringValue(item.path) || stringValue(item.filename) || `file-${index + 1}`,
      status: stringValue(item.status) || stringValue(item.kind),
      additions: typeof item.additions === 'number' ? item.additions : null,
      deletions: typeof item.deletions === 'number' ? item.deletions : null,
    }
  })
})

const mediaUrl = computed(() => stringValue(props.block.url) || stringValue(jsonRecord(props.block.source).url))
const mediaMime = computed(() => stringValue(props.block.mime) || stringValue(props.block.media_type))
const artifact = computed(() => jsonRecord(props.block.artifact))
const resourceUri = computed(
  () =>
    stringValue(props.block.uri) ||
    stringValue(props.block.url) ||
    stringValue(artifact.value.uri) ||
    stringValue(jsonRecord(props.block.source).url),
)
const resourceTitle = computed(
  () =>
    stringValue(props.block.title) ||
    stringValue(props.block.filename) ||
    stringValue(artifact.value.name) ||
    resourceUri.value,
)
const resourceMime = computed(
  () => stringValue(props.block.mime) || stringValue(props.block.mime_type) || stringValue(artifact.value.mime),
)
const checklistItems = computed(() =>
  jsonArray(props.block.items).map((raw, index) => {
    const item = jsonRecord(raw)
    return {
      key: stringValue(item.id) || `${stringValue(item.content)}:${index}`,
      content: stringValue(item.content) || stringValue(item.title) || `Item ${index + 1}`,
      status: stringValue(item.status),
      priority: stringValue(item.priority),
    }
  }),
)
const progressPercent = computed(() =>
  typeof props.block.percent === 'number' && Number.isFinite(props.block.percent)
    ? Math.max(0, Math.min(100, props.block.percent))
    : null,
)
</script>

<template>
  <div class="min-w-0 text-[13px] leading-relaxed">
    <MarkdownRenderer v-if="kind === 'markdown'" :content="bodyText" mode="markdown" :stream="false" />

    <pre
      v-else-if="kind === 'text' || kind === 'log'"
      class="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground/90"
      >{{ bodyText }}</pre
    >

    <div v-else-if="kind === 'command'" class="space-y-2">
      <div v-if="cwd" class="font-mono text-[10px] text-muted-foreground">{{ cwd }}</div>
      <div class="font-mono text-xs text-foreground"><span class="text-primary">$</span> {{ command }}</div>
      <CodeBlock v-if="stdout" :code="stdout" lang="text" compact />
      <CodeBlock v-if="stderr" :code="stderr" lang="text" compact />
      <div v-if="exitCode !== null" class="font-mono text-[11px] text-muted-foreground">exit {{ exitCode }}</div>
    </div>

    <CodeBlock
      v-else-if="kind === 'json'"
      :code="prettyJson(block.value ?? block.data ?? block.content ?? block)"
      lang="json"
      compact
    />

    <CodeBlock v-else-if="kind === 'diff'" :code="stringValue(block.diff) || bodyText" lang="diff" compact />

    <div v-else-if="kind === 'table'" class="overflow-x-auto border-y border-border/60">
      <table class="w-full min-w-max border-collapse text-left font-mono text-xs">
        <thead class="text-muted-foreground">
          <tr>
            <th
              v-for="column in tableColumns"
              :key="column.key"
              class="border-b border-border/60 px-2 py-1.5 font-medium"
            >
              {{ column.label }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, rowIndex) in tableRows" :key="rowIndex" class="border-b border-border/35 last:border-b-0">
            <td v-for="(column, columnIndex) in tableColumns" :key="column.key" class="px-2 py-1.5 align-top">
              {{ tableCell(row, column.key, columnIndex) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-else-if="kind === 'search_results'" class="divide-y divide-border/40 border-y border-border/60">
      <div v-for="result in searchResults" :key="result.key" class="py-2 first:pt-1 last:pb-1">
        <a
          v-if="result.uri"
          :href="result.uri"
          target="_blank"
          rel="noopener noreferrer"
          class="font-medium text-primary hover:underline"
          >{{ result.title }}</a
        >
        <div v-else class="font-medium">{{ result.title }}</div>
        <div v-if="result.snippet" class="mt-0.5 text-xs text-muted-foreground">{{ result.snippet }}</div>
        <div v-if="result.uri" class="mt-0.5 truncate font-mono text-[10px] text-muted-foreground/75">
          {{ result.uri }}
        </div>
      </div>
    </div>

    <div
      v-else-if="kind === 'file_changes'"
      class="divide-y divide-border/40 border-y border-border/60 font-mono text-xs"
    >
      <div v-for="change in fileChanges" :key="change.key" class="flex min-w-0 items-center gap-3 py-1.5">
        <span class="min-w-0 flex-1 truncate">{{ change.path }}</span>
        <span v-if="change.status" class="text-muted-foreground">{{ change.status }}</span>
        <span v-if="change.additions !== null" class="text-emerald-600 dark:text-emerald-400"
          >+{{ change.additions }}</span
        >
        <span v-if="change.deletions !== null" class="text-rose-600 dark:text-rose-400">-{{ change.deletions }}</span>
      </div>
    </div>

    <div v-else-if="kind === 'media' && mediaUrl" class="space-y-2">
      <img v-if="mediaMime.startsWith('image/')" :src="mediaUrl" alt="" class="max-h-96 max-w-full object-contain" />
      <video v-else-if="mediaMime.startsWith('video/')" :src="mediaUrl" controls class="max-h-96 max-w-full" />
      <audio v-else-if="mediaMime.startsWith('audio/')" :src="mediaUrl" controls class="w-full" />
      <a
        v-else
        :href="mediaUrl"
        target="_blank"
        rel="noopener noreferrer"
        class="font-mono text-xs text-primary hover:underline"
      >
        {{ mediaUrl }}
      </a>
    </div>

    <div v-else-if="kind === 'image' || kind === 'audio' || kind === 'file' || kind === 'media'" class="space-y-2">
      <img
        v-if="resourceMime.startsWith('image/') && resourceUri"
        :src="resourceUri"
        :alt="resourceTitle"
        class="max-h-96 max-w-full object-contain"
      />
      <audio v-else-if="resourceMime.startsWith('audio/') && resourceUri" :src="resourceUri" controls class="w-full" />
      <a
        v-else-if="resourceUri"
        :href="resourceUri"
        target="_blank"
        rel="noopener noreferrer"
        class="break-all font-mono text-xs text-primary hover:underline"
      >
        {{ resourceTitle }}
      </a>
    </div>

    <div v-else-if="kind === 'resource_link' || kind === 'citation'" class="border-y border-border/50 py-2">
      <a
        :href="resourceUri || undefined"
        target="_blank"
        rel="noopener noreferrer"
        class="break-words font-medium text-primary hover:underline"
      >
        {{ resourceTitle }}
      </a>
      <div v-if="bodyText" class="mt-1 text-xs text-muted-foreground">{{ bodyText }}</div>
      <div v-if="resourceUri" class="mt-1 break-all font-mono text-[10px] text-muted-foreground/75">
        {{ resourceUri }}
      </div>
    </div>

    <div v-else-if="kind === 'embedded_resource'" class="space-y-2 border-y border-border/50 py-2">
      <div class="break-all font-mono text-[11px] text-muted-foreground">{{ resourceTitle }}</div>
      <pre v-if="bodyText" class="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs">{{
        bodyText
      }}</pre>
      <CodeBlock v-else :code="prettyJson(block)" lang="json" compact />
    </div>

    <div v-else-if="kind === 'checklist'" class="divide-y divide-border/40 border-y border-border/60 text-xs">
      <div v-for="item in checklistItems" :key="item.key" class="flex min-w-0 items-start gap-2 py-1.5">
        <span class="shrink-0 font-mono text-primary">{{ item.status === 'completed' ? '[x]' : '[ ]' }}</span>
        <span class="min-w-0 flex-1 break-words">{{ item.content }}</span>
        <span v-if="item.priority" class="shrink-0 font-mono text-[10px] text-muted-foreground">{{
          item.priority
        }}</span>
      </div>
    </div>

    <div v-else-if="kind === 'progress'" class="space-y-1.5 border-y border-border/50 py-2">
      <div class="flex items-center justify-between gap-3 text-xs">
        <span class="min-w-0 break-words">{{ stringValue(block.message) }}</span>
        <span v-if="progressPercent !== null" class="shrink-0 font-mono text-[10px] text-muted-foreground">
          {{ progressPercent }}%
        </span>
      </div>
      <div v-if="progressPercent !== null" class="h-1 overflow-hidden bg-muted">
        <div class="h-full bg-primary" :style="{ width: `${progressPercent}%` }" />
      </div>
    </div>

    <div
      v-else-if="kind === 'nested_task'"
      class="flex min-w-0 items-center gap-2 border-y border-border/50 py-2 text-xs"
    >
      <span class="font-mono text-primary">{{ stringValue(block.status) || 'pending' }}</span>
      <span class="min-w-0 flex-1 break-words">{{ stringValue(block.title) || stringValue(block.task_id) }}</span>
    </div>

    <CodeBlock v-else-if="kind === 'custom'" :code="prettyJson(block.value)" lang="json" compact />

    <CodeBlock v-else :code="prettyJson(block.value ?? block.presentation ?? block)" lang="json" compact />
  </div>
</template>
