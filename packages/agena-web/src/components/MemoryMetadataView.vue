<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { MemoryMetadata } from '@/generated/agenaState'
import { settingsText as st } from '@/i18n/settingsText'

const props = withDefaults(defineProps<{ metadata?: MemoryMetadata; compact?: boolean; timezone?: string }>(), {
  compact: false,
})
const { locale } = useI18n()
const status = computed(() => props.metadata?.status ?? 'active')
const statusLabel = computed(() => {
  if (props.metadata?.source_suppressed) return st('Source forgotten')
  if (status.value === 'superseded') return st('Superseded')
  if (status.value === 'archived') return st('Archived')
  return st('Active')
})
const originLabel = computed(() => {
  if (props.metadata?.origin === 'derived') return st('Learned from a task')
  if (props.metadata?.origin === 'explicit') return st('Saved explicitly')
  return st('Imported or edited')
})
const updated = computed(() => {
  const ms = props.metadata?.updated_at_ms
  if (!ms || !Number.isFinite(ms)) return null
  const date = new Date(ms)
  if (!Number.isFinite(date.getTime())) return null
  try {
    return { iso: date.toISOString(), label: date.toLocaleString(String(locale.value), { timeZone: props.timezone }) }
  } catch {
    return { iso: date.toISOString(), label: date.toLocaleString(String(locale.value)) }
  }
})
</script>

<template>
  <span v-if="metadata" class="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
    <span
      class="rounded-md px-1.5 py-0.5 font-medium"
      :class="status === 'active' ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground'"
      :data-memory-status="status"
      >{{ statusLabel }}</span
    >
    <span v-if="!compact">{{ originLabel }}</span>
    <time v-if="updated" :datetime="updated.iso">{{ st('Updated {time}', { time: updated.label }) }}</time>
    <span v-if="!compact && metadata.scope?.kind === 'session'">
      {{ st('Applies to session {id}', { id: metadata.scope.session_id }) }}
    </span>
    <span v-if="!compact && metadata.source?.session_id != null" class="break-all">
      {{ st('Source') }}: {{ st('Session {id}', { id: metadata.source.session_id }) }}
      <span v-if="metadata.source.entry_id != null"> · {{ st('Entry {id}', { id: metadata.source.entry_id }) }}</span>
    </span>
  </span>
</template>
