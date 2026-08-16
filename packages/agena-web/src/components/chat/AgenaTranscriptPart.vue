<script setup lang="ts">
import { computed } from 'vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import CodeBlock from '@/components/ui/CodeBlock.vue'
import AttentionPanel from '@/components/chat/AttentionPanel.vue'
import AgenaOperationPart from '@/components/chat/AgenaOperationPart.vue'
import type { AttentionLike, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import { durablePartContent, transcriptPartText } from '@/pages/chat/transcriptProjection'
import {
  attachmentPresentations,
  errorPresentation,
  interactionPresentation,
  partStatusPresentation,
  prettyJson,
  skillPresentations,
  attentionRequestId,
} from '@/pages/chat/transcriptPartPresentation'
import { buildWorkspaceRawFileUrl } from '@/lib/workspaceLinks'
import { useDirectoryStore } from '@/stores/directory'
import { useUiStore } from '@/stores/ui'

const props = defineProps<{
  part: TranscriptDisplayPart
  expanded: boolean
  collapseSignal: number
  streaming?: boolean
  sourcePath?: string
  sessionId?: string | null
  attention?: AttentionLike
}>()

const emit = defineEmits<{
  (event: 'toggle'): void
  (event: 'select'): void
}>()

const directory = useDirectoryStore()
const ui = useUiStore()
const status = computed(() => partStatusPresentation(props.part.status))
const body = computed(() => transcriptPartText(props.part.source))
const content = computed(() => durablePartContent(props.part.source))
const attachments = computed(() => attachmentPresentations(props.part))
const skills = computed(() => skillPresentations(props.part))
const interaction = computed(() => interactionPresentation(props.part))
const failure = computed(() => errorPresentation(props.part))
const activeAttentionRequestId = computed(() => attentionRequestId(props.attention?.payload))
const interactionOwnsAttention = computed(
  () =>
    Boolean(interaction.value.pending && interaction.value.requestId) &&
    interaction.value.requestId === activeAttentionRequestId.value,
)

function toggle() {
  emit('select')
  emit('toggle')
}

function attachmentUrl(path: string, url: string): string {
  const workspace = String(directory.currentDirectory || '').trim()
  if (workspace && path && !path.startsWith('http://') && !path.startsWith('https://') && !path.startsWith('data:')) {
    return buildWorkspaceRawFileUrl(workspace, path)
  }
  return url || path
}

function openAttachment(path: string, url: string) {
  const workspace = String(directory.currentDirectory || '').trim()
  if (workspace && path) {
    ui.requestWorkspaceDockFile(path, 'open')
    return
  }
  if (url) window.open(url, '_blank', 'noopener,noreferrer')
}

function isImage(mime: string, label: string): boolean {
  return mime.startsWith('image/') || /\.(png|jpe?g|gif|webp|avif|svg)$/i.test(label)
}

function isVideo(mime: string, label: string): boolean {
  return mime.startsWith('video/') || /\.(mp4|webm|mov|m4v)$/i.test(label)
}

function isAudio(mime: string, label: string): boolean {
  return mime.startsWith('audio/') || /\.(mp3|wav|m4a|ogg|flac)$/i.test(label)
}
</script>

<template>
  <AgenaOperationPart
    v-if="part.kind === 'operation'"
    :part="part"
    :expanded="expanded"
    :collapse-signal="collapseSignal"
    :session-id="sessionId"
    :attention="attention"
    @toggle="$emit('toggle')"
    @select="$emit('select')"
  />

  <div v-else-if="part.kind === 'text'" class="min-w-0 py-1 pl-7 text-sm leading-relaxed">
    <MarkdownRenderer :content="body" mode="markdown" :stream="Boolean(streaming)" :source-path="sourcePath || ''" />
  </div>

  <div v-else-if="part.kind === 'lifecycle'" class="flex min-w-0 items-baseline gap-2 py-1 pl-3">
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
    <span class="text-[13px] font-semibold" :class="status.tone === 'danger' ? 'text-rose-700 dark:text-rose-300' : ''">
      {{ part.title }}
    </span>
  </div>

  <div v-else class="min-w-0">
    <button
      type="button"
      class="group/headline flex w-full min-w-0 items-baseline gap-2 py-1 text-left outline-none"
      :aria-expanded="expanded"
      data-transcript-vim-toggle="true"
      @click="toggle"
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
      <span
        class="min-w-0 flex-1 truncate text-[13px] font-semibold"
        :class="part.kind === 'error' ? 'text-rose-700 dark:text-rose-300' : ''"
        >{{ part.title }}</span
      >
      <span v-if="!expanded && part.summary" class="min-w-0 max-w-[60%] truncate text-xs text-muted-foreground"
        >· {{ part.summary }}</span
      >
    </button>

    <div v-if="expanded" class="ml-5 min-w-0 border-l border-border/60 pb-1 pl-4">
      <div v-if="part.kind === 'answer' || part.kind === 'text_segment'" class="py-1 text-sm leading-relaxed">
        <MarkdownRenderer
          :content="body"
          mode="markdown"
          :stream="Boolean(streaming)"
          :source-path="sourcePath || ''"
        />
      </div>

      <pre
        v-else-if="part.kind === 'reasoning'"
        class="overflow-x-auto whitespace-pre-wrap break-words py-1 font-mono text-xs leading-relaxed text-muted-foreground"
        >{{ body }}</pre
      >

      <div v-else-if="part.kind === 'resource'" class="space-y-2 py-1">
        <div v-for="attachment in attachments" :key="attachment.key" class="min-w-0">
          <button
            type="button"
            class="flex w-full min-w-0 items-center gap-2 py-1 text-left font-mono text-xs hover:text-primary"
            @click="openAttachment(attachment.path, attachment.url)"
          >
            <span aria-hidden="true">›</span>
            <span class="min-w-0 flex-1 truncate">{{ attachment.label }}</span>
            <span v-if="attachment.mime" class="text-[10px] text-muted-foreground">{{ attachment.mime }}</span>
          </button>
          <img
            v-if="isImage(attachment.mime, attachment.label) && attachmentUrl(attachment.path, attachment.url)"
            :src="attachmentUrl(attachment.path, attachment.url)"
            :alt="attachment.label"
            class="mt-1 max-h-96 max-w-full cursor-zoom-in object-contain"
            @click="openAttachment(attachment.path, attachment.url)"
          />
          <video
            v-else-if="isVideo(attachment.mime, attachment.label) && attachmentUrl(attachment.path, attachment.url)"
            :src="attachmentUrl(attachment.path, attachment.url)"
            controls
            preload="metadata"
            class="mt-1 max-h-96 max-w-full"
          />
          <audio
            v-else-if="isAudio(attachment.mime, attachment.label) && attachmentUrl(attachment.path, attachment.url)"
            :src="attachmentUrl(attachment.path, attachment.url)"
            controls
            preload="metadata"
            class="mt-1 w-full"
          />
        </div>
      </div>

      <div v-else-if="part.kind === 'skill'" class="divide-y divide-border/50 py-1">
        <section v-for="skill in skills" :key="skill.name" class="py-2 first:pt-1 last:pb-1">
          <div class="font-mono text-xs font-semibold">{{ skill.name }}</div>
          <div v-if="skill.description" class="mt-1 text-xs text-muted-foreground">{{ skill.description }}</div>
          <div v-if="skill.instructions" class="mt-2">
            <div class="mb-1 text-xs font-semibold text-primary">› Instructions</div>
            <MarkdownRenderer :content="skill.instructions" mode="markdown" :stream="false" />
          </div>
          <div v-if="skill.source || skill.contentHash" class="mt-2 font-mono text-[10px] text-muted-foreground">
            {{ [skill.source, skill.contentHash].filter(Boolean).join(' · ') }}
          </div>
        </section>
      </div>

      <div v-else-if="part.kind === 'interaction'" class="space-y-3 py-1 text-sm">
        <AttentionPanel
          v-if="interactionOwnsAttention && attention && sessionId"
          :kind="attention.kind"
          :session-id="sessionId"
          :payload="attention.payload"
          inline
        />
        <template v-else>
          <MarkdownRenderer
            v-if="interaction.bodyMarkdown"
            :content="interaction.bodyMarkdown"
            mode="markdown"
            :stream="false"
          />
          <section
            v-for="(question, questionIndex) in interaction.questions"
            :key="`${question.question}:${questionIndex}`"
          >
            <div v-if="question.header" class="text-[10px] font-semibold uppercase text-muted-foreground">
              {{ question.header }}
            </div>
            <div class="font-medium">{{ question.question }}</div>
            <div class="mt-1 border-y border-border/50">
              <div v-for="option in question.options" :key="option.label" class="py-1.5 text-xs">
                <span class="font-mono text-primary">○</span>
                <span class="ml-2 font-medium">{{ option.label }}</span>
                <span v-if="option.description" class="ml-2 text-muted-foreground">{{ option.description }}</span>
              </div>
            </div>
          </section>
          <div v-if="interaction.pending" class="font-mono text-[11px] text-amber-600 dark:text-amber-400">
            Awaiting user input
          </div>
          <CodeBlock v-else :code="prettyJson(interaction.reply)" lang="json" compact />
        </template>
      </div>

      <div v-else-if="part.kind === 'error'" class="space-y-2 py-1 text-sm text-rose-800 dark:text-rose-200">
        <div>{{ failure.message }}</div>
        <dl class="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 font-mono text-[11px]">
          <template
            v-for="item in [
              ['code', failure.code],
              ['category', failure.category],
              ['responsibility', failure.responsibility],
              ['impact', failure.impact],
              ['recovery', failure.recovery],
              ['retry', failure.retry],
              ['correlation', failure.correlationId],
            ].filter((entry) => entry[1])"
            :key="item[0]"
          >
            <dt class="text-rose-700/70 dark:text-rose-300/70">{{ item[0] }}</dt>
            <dd class="min-w-0 break-words">{{ item[1] }}</dd>
          </template>
        </dl>
      </div>

      <div v-else-if="part.kind === 'notice' || part.kind === 'compaction'" class="py-1 text-sm">
        <MarkdownRenderer
          :content="String(content.detail || content.message || content.body || content.summary || part.summary || '')"
          mode="markdown"
          :stream="false"
        />
      </div>

      <CodeBlock v-else :code="prettyJson(content)" lang="json" compact />
    </div>
  </div>
</template>
