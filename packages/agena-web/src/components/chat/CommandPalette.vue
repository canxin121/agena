<script setup lang="ts">
import type { Component } from 'vue'
import { useI18n } from 'vue-i18n'
import type { Command } from '@/pages/chat/useChatCommands'
import ComposerPalette from './ComposerPalette.vue'

defineProps<{
  open: boolean
  autoFocus: boolean
  loading: boolean
  query: string
  commands: Command[]
  activeIndex: number
  commandIcon: (command: Command) => Component
  listId?: string
}>()
const emit = defineEmits<{
  'update:activeIndex': [index: number]
  'update:query': [query: string]
  keydown: [event: KeyboardEvent]
  select: [command: Command]
  dismiss: []
}>()
const { t } = useI18n()
</script>

<template>
  <ComposerPalette
    kind="command"
    :open="open"
    :auto-focus="autoFocus"
    :loading="loading"
    :query="query"
    :items="commands"
    :item-key="(command) => command.name"
    :active-index="activeIndex"
    :list-id="listId"
    :search-label="t('chat.commandPalette.search')"
    :list-label="t('chat.commandPalette.search')"
    :empty-text="t('chat.commandPalette.empty')"
    :hint="t('chat.commandPalette.hint')"
    @update:query="emit('update:query', $event)"
    @update:active-index="emit('update:activeIndex', $event)"
    @keydown="emit('keydown', $event)"
    @select="emit('select', $event)"
    @dismiss="emit('dismiss')"
  >
    <template #icon><span class="font-mono text-sm text-primary" aria-hidden="true">/</span></template>
    <template #item="{ item: cmd }">
      <component :is="commandIcon(cmd)" class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <div class="min-w-0 flex-1">
        <div class="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span class="min-w-0 truncate font-mono text-sm">/{{ cmd.name }}</span>
          <span v-if="cmd.arguments" class="min-w-0 truncate font-mono text-[11px] text-muted-foreground">{{
            cmd.arguments
          }}</span>
          <span
            v-if="cmd.kind === 'builtin'"
            class="text-[10px] uppercase font-bold tracking-tight rounded border border-amber-300/40 bg-amber-200/10 text-amber-600 px-1.5 py-0.5"
          >
            {{ t('chat.roles.system') }}
          </span>
          <span
            v-else-if="cmd.scope"
            class="text-[10px] uppercase font-bold tracking-tight rounded border border-emerald-300/40 bg-emerald-200/10 text-emerald-600 px-1.5 py-0.5"
          >
            {{ cmd.scope }}
          </span>
        </div>
        <div v-if="cmd.description" class="text-xs text-muted-foreground truncate">{{ cmd.description }}</div>
        <div v-if="cmd.aliases?.length" class="text-xs text-muted-foreground truncate">
          {{ t('chat.commandPalette.aliases') }}: /{{ cmd.aliases.join(' /') }}
        </div>
      </div>
    </template>
  </ComposerPalette>
</template>
