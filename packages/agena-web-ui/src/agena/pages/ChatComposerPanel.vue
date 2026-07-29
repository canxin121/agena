<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { sourceLabel, type CommandItem } from '@/agena/lib/commandPalette'
import { formatComposerAttachmentSize, type ComposerAttachmentDraft } from './chatAttachmentModel'
import { composerQueuePreview, type ComposerQueueItem } from './chatQueueModel'
import type { ComposerSkillDraft } from './chatSkillModel'

const props = defineProps<{
  attachments: ComposerAttachmentDraft[]
  skills: ComposerSkillDraft[]
  attachmentLoading: boolean
  addFiles: (files: File[], imageOnly?: boolean) => void | Promise<void>
  composer: string
  queue: ComposerQueueItem[]
  slashSuggestions: CommandItem[]
  sending: boolean
  openPalette: () => void
  openSkillPicker: () => void
  sendPrompt: () => void | Promise<void>
  removeAttachment: (id: string) => void
  removeSkill: (id: string) => void
  clearQueue: () => void
  popQueue: () => void
}>()

const emit = defineEmits<{
  'update:composer': [value: string]
}>()

const highlightedIndex = ref(0)

const slashSearchActive = computed(() => props.composer.trimStart().startsWith('/'))
const selectedSuggestion = computed(
  () => props.slashSuggestions[highlightedIndex.value] || props.slashSuggestions[0] || null,
)
const canSubmit = computed(() => {
  const text = props.composer.trim()
  return Boolean(text || props.attachments.length || props.skills.length)
})

function selectFiles(event: Event, imageOnly: boolean) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files || [])
  input.value = ''
  if (files.length) void props.addFiles(files, imageOnly)
}

watch(
  () => [props.composer, props.slashSuggestions.length] as const,
  () => {
    if (highlightedIndex.value >= props.slashSuggestions.length) {
      highlightedIndex.value = Math.max(0, props.slashSuggestions.length - 1)
    }
  },
)

function commandSourceLabel(item: CommandItem): string {
  return item.sourceLabel || sourceLabel(item.source)
}

function commandCompletion(item: CommandItem): string {
  const slash = item.slash || props.composer
  const current = props.composer.trimStart()
  const currentSlash = (current.split(/\s+/)[0] || '').toLowerCase()
  if (currentSlash === slash.toLowerCase() && /\s/.test(current)) {
    return props.composer
  }
  const usage = item.usage || ''
  const expectsArgs = usage.includes('<') || usage.includes('[')
  return expectsArgs ? `${slash} ` : slash
}

function chooseSuggestion(item: CommandItem | null) {
  if (!item) return
  if (item.executeOnSelect && item.slash) {
    emit('update:composer', '')
    void item.run({ input: item.slash, args: [] })
    return
  }
  emit('update:composer', commandCompletion(item))
}

function moveSuggestion(delta: number) {
  if (!props.slashSuggestions.length) return
  highlightedIndex.value =
    (highlightedIndex.value + delta + props.slashSuggestions.length) % props.slashSuggestions.length
}

function handleComposerKeydown(event: KeyboardEvent) {
  if (!props.slashSuggestions.length) return

  if (event.key === 'ArrowDown') {
    event.preventDefault()
    moveSuggestion(1)
    return
  }

  if (event.key === 'ArrowUp') {
    event.preventDefault()
    moveSuggestion(-1)
    return
  }

  if (event.key === 'Tab') {
    event.preventDefault()
    chooseSuggestion(selectedSuggestion.value)
    return
  }

  if (event.key === 'Enter' && !event.shiftKey) {
    const currentSlash = (props.composer.trimStart().split(/\s+/)[0] || '').toLowerCase()
    const selectedSlash = (selectedSuggestion.value?.slash || '').toLowerCase()
    if (currentSlash && currentSlash === selectedSlash && selectedSuggestion.value?.executeOnSelect) {
      event.preventDefault()
      chooseSuggestion(selectedSuggestion.value)
      return
    }
    if (currentSlash && selectedSlash && currentSlash !== selectedSlash) {
      event.preventDefault()
      chooseSuggestion(selectedSuggestion.value)
    }
  }
}
</script>

<template>
  <section class="card">
    <div class="page-header" style="margin-bottom: 12px">
      <h3 style="margin: 0">Composer</h3>
      <div class="button-row">
        <button class="button ghost" @click="props.openPalette">Open Palette</button>
      </div>
    </div>
    <div class="field">
      <label class="label" for="composer">Prompt</label>
      <textarea
        id="composer"
        :value="props.composer"
        class="textarea mono"
        placeholder="Ask agena to inspect the repo, plan a change, or run tools. Try /runtime or /new-session."
        aria-controls="composer-slash-candidates"
        :aria-expanded="slashSearchActive"
        @input="emit('update:composer', ($event.target as HTMLTextAreaElement | null)?.value || '')"
        @keydown="handleComposerKeydown"
      />
    </div>
    <input id="composer-file-input" class="visually-hidden" type="file" multiple @change="selectFiles($event, false)" />
    <input
      id="composer-image-input"
      class="visually-hidden"
      type="file"
      accept="image/*"
      multiple
      @change="selectFiles($event, true)"
    />
    <div v-if="props.attachments.length" class="composer-attachment-list">
      <article v-for="attachment in props.attachments" :key="attachment.id" class="composer-attachment-chip">
        <div>
          <strong>{{ attachment.name }}</strong>
          <div class="muted mono">{{ attachment.kind }} · {{ formatComposerAttachmentSize(attachment.size) }}</div>
        </div>
        <button
          class="button ghost"
          :disabled="props.sending || props.attachmentLoading"
          @click="props.removeAttachment(attachment.id)"
        >
          Remove
        </button>
      </article>
    </div>
    <div v-if="props.skills.length" class="composer-attachment-list">
      <article v-for="skill in props.skills" :key="skill.id" class="composer-attachment-chip composer-skill-chip">
        <div>
          <div class="composer-skill-title">
            <span class="badge">Skill</span><strong>{{ skill.name }}</strong>
          </div>
          <div class="muted">{{ skill.description || 'User-selected Skill instructions' }}</div>
          <div class="muted mono">{{ skill.source }} · {{ skill.contentHash.slice(0, 12) }}</div>
        </div>
        <button
          class="button ghost"
          :disabled="props.sending || props.attachmentLoading"
          @click="props.removeSkill(skill.id)"
        >
          Remove
        </button>
      </article>
    </div>
    <div v-if="props.queue.length" class="composer-queue">
      <div class="settings-panel-header">
        <div>
          <strong>Pending Messages</strong>
          <div class="muted">Sent automatically in order after the active run becomes idle.</div>
        </div>
        <span class="badge warn">{{ props.queue.length }} queued</span>
      </div>
      <ol class="composer-queue-list">
        <li v-for="item in props.queue" :key="item.id">
          <span class="mono">{{ composerQueuePreview(item) }}</span>
          <span v-if="item.attachments.length" class="badge neutral">{{ item.attachments.length }} file(s)</span>
          <span v-if="item.skills.length" class="badge">{{ item.skills.length }} Skill(s)</span>
        </li>
      </ol>
      <div class="button-row">
        <button class="button" :disabled="props.sending" @click="props.popQueue">Edit First</button>
        <button class="button danger" :disabled="props.sending" @click="props.clearQueue">Clear Queue</button>
      </div>
    </div>
    <div v-if="props.slashSuggestions.length" id="composer-slash-candidates" class="slash-command-menu" role="listbox">
      <div class="slash-command-menu-head">
        <strong>Slash Commands</strong>
        <span class="badge">{{ props.slashSuggestions.length }}</span>
      </div>
      <button
        v-for="(item, index) in props.slashSuggestions"
        :key="item.id"
        class="slash-command-item"
        :class="{ active: index === highlightedIndex }"
        role="option"
        :aria-selected="index === highlightedIndex"
        @mouseenter="highlightedIndex = index"
        @mousedown.prevent
        @click="chooseSuggestion(item)"
      >
        <div class="slash-command-main">
          <div class="slash-command-title">
            <span class="badge mono">{{ item.slash }}</span>
            <strong>{{ item.title }}</strong>
          </div>
          <div class="muted">{{ item.description }}</div>
        </div>
        <div class="slash-command-meta">
          <span>{{ item.category }}</span>
          <span class="muted mono">{{ item.usage || item.slash }}</span>
          <span class="muted">{{ commandSourceLabel(item) }}</span>
        </div>
      </button>
    </div>
    <div v-else-if="slashSearchActive" id="composer-slash-candidates" class="slash-command-menu empty">
      <span class="muted">No slash commands matched.</span>
    </div>
    <div class="button-row" style="margin-top: 12px">
      <label class="button" for="composer-file-input">Attach File</label>
      <label class="button" for="composer-image-input">Attach Image</label>
      <button class="button" type="button" @click="props.openSkillPicker">Attach Skill</button>
      <button
        class="button primary"
        :disabled="props.sending || props.attachmentLoading || !canSubmit"
        @click="props.sendPrompt"
      >
        {{ props.sending ? 'Sending…' : props.attachmentLoading ? 'Reading files…' : 'Send Prompt' }}
      </button>
    </div>
  </section>
</template>
