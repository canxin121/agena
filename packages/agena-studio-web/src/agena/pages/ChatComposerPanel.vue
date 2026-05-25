<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { sourceLabel, type CommandItem } from '@/agena/lib/commandPalette'

const props = defineProps<{
  composer: string
  slashSuggestions: CommandItem[]
  sending: boolean
  openPalette: () => void
  sendPrompt: () => void | Promise<void>
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
  return Boolean(text)
})

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
      <button class="button primary" :disabled="props.sending || !canSubmit" @click="props.sendPrompt">
        {{ props.sending ? 'Sending…' : 'Send Prompt' }}
      </button>
    </div>
  </section>
</template>
