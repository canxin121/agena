<script setup lang="ts">
import type { CommandItem } from '@/agena/lib/commandPalette'

const props = defineProps<{
  composer: string
  slashSuggestions: CommandItem[]
  sending: boolean
  selectedSessionId: number | null
  openPalette: () => void
  sendPrompt: () => void | Promise<void>
}>()

const emit = defineEmits<{
  'update:composer': [value: string]
}>()
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
        @input="emit('update:composer', ($event.target as HTMLTextAreaElement | null)?.value || '')"
      />
    </div>
    <div v-if="props.slashSuggestions.length" class="list" style="margin-top: 12px">
      <button
        v-for="item in props.slashSuggestions"
        :key="item.id"
        class="list-item"
        style="width: 100%; text-align: left"
        @click="emit('update:composer', item.slash || props.composer)"
      >
        <div class="page-header" style="align-items: flex-start">
          <div>
            <strong>{{ item.title }}</strong>
            <div class="muted">{{ item.description }}</div>
            <div v-if="item.sourceLabel" class="muted mono">source={{ item.sourceLabel }}</div>
            <div v-if="item.usage" class="muted mono">{{ item.usage }}</div>
          </div>
          <span class="badge">{{ item.slash }}</span>
        </div>
      </button>
    </div>
    <div class="button-row" style="margin-top: 12px">
      <button
        class="button primary"
        :disabled="props.sending || !props.selectedSessionId || !props.composer.trim()"
        @click="props.sendPrompt"
      >
        {{ props.sending ? 'Sending…' : 'Send Prompt' }}
      </button>
    </div>
  </section>
</template>
