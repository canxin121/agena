<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { MemoryMetadata } from '@/generated/agenaState'
import { settingsText as st } from '@/i18n/settingsText'

const props = withDefaults(defineProps<{ metadata: MemoryMetadata; compact?: boolean; timezone?: string }>(), {
  compact: false,
})
const { locale } = useI18n()
const kindLabel = computed(
  () =>
    ({
      fact: st('Fact'),
      preference: st('Preference'),
      episode: st('Episode'),
      procedure: st('Procedure'),
      task: st('Task'),
    })[props.metadata.kind],
)
const status = computed(() => props.metadata.status)
const statusLabel = computed(() => {
  if (props.metadata.source_suppressed) return st('Source forgotten')
  return { active: st('Active'), superseded: st('Superseded'), archived: st('Archived') }[status.value]
})
const originLabel = computed(
  () =>
    ({
      derived: st('Learned from a task'),
      explicit: st('Saved explicitly'),
      external: st('Edited externally'),
    })[props.metadata.origin],
)
const updated = computed(() => {
  const ms = props.metadata.updated_at_ms
  if (!Number.isFinite(ms)) return null
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
  <span class="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
    <span
      class="rounded-md bg-secondary px-1.5 py-0.5 font-medium text-secondary-foreground"
      :data-memory-kind="metadata.kind"
      >{{ kindLabel }}</span
    >
    <span
      class="rounded-md px-1.5 py-0.5 font-medium"
      :class="status === 'active' ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground'"
      :data-memory-status="status"
      >{{ statusLabel }}</span
    >
    <span v-if="!compact">{{ originLabel }}</span>
    <time v-if="updated" :datetime="updated.iso">{{ st('Updated {time}', { time: updated.label }) }}</time>
    <span v-if="!compact && metadata.scope.kind === 'session'">
      {{ st('Applies to session {id}', { id: metadata.scope.session_id }) }}
    </span>
    <span v-if="!compact && metadata.source" class="break-all">
      {{ st('Source') }}: {{ st('Session {id}', { id: metadata.source.session_id }) }}
      <span v-if="metadata.source.entry_id != null"> · {{ st('Entry {id}', { id: metadata.source.entry_id }) }}</span>
    </span>
  </span>
</template>
