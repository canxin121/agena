<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiResetLeftLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import type { PluginRuntimeStatus } from '@/features/pluginWeb/hub'
import SettingsToggle from '../SettingsToggle.vue'
import { pluginStateLabel } from '../pluginLabels'

const props = defineProps<{
  plugins: PluginRuntimeStatus[]
  local?: Record<string, boolean>
  inherited?: Record<string, boolean>
  editable: boolean
  disabled: boolean
  loadFailed: boolean
}>()
const emit = defineEmits<{ change: [id: string, enabled: boolean | undefined]; retry: [] }>()
const search = ref('')
const onlyDisabled = ref(false)
const rows = computed(() => {
  const installed = new Map(props.plugins.map((plugin) => [plugin.plugin_id, plugin]))
  const ids = new Set([...installed.keys(), ...Object.keys(props.inherited || {}), ...Object.keys(props.local || {})])
  const query = search.value.trim().toLocaleLowerCase()
  return [...ids]
    .map((id) => {
      const status = installed.get(id)
      return {
        id,
        title: status?.display_name || id,
        runtimeState: status?.state === 'running' ? '' : status?.state,
        enabled: props.local?.[id] ?? props.inherited?.[id] ?? true,
        overridden: Object.hasOwn(props.local || {}, id),
      }
    })
    .filter((row) => {
      return (
        (!onlyDisabled.value || !row.enabled) &&
        (!query || [row.title, row.id].some((value) => value.toLocaleLowerCase().includes(query)))
      )
    })
    .sort((left, right) => left.title.localeCompare(right.title) || left.id.localeCompare(right.id))
})
</script>

<template>
  <div class="space-y-2 px-3 py-2.5" data-permission-plugin-access>
    <div>
      <h3 class="text-sm font-medium">{{ $st('Plugin access') }}</h3>
      <p class="text-xs text-muted-foreground">
        {{ $st('Switch an entire plugin’s tools on or off for this permission configuration.') }}
      </p>
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <SearchInput
        v-model="search"
        class="min-w-32 flex-1"
        input-class="h-8 text-xs"
        :placeholder="$st('Search plugins')"
        :input-aria-label="$st('Search plugins')"
        :show-search-button="false"
      />
      <Button
        type="button"
        size="sm"
        :variant="onlyDisabled ? 'secondary' : 'outline'"
        :aria-pressed="onlyDisabled"
        @click="onlyDisabled = !onlyDisabled"
        >{{ $st('Disabled only') }}</Button
      >
    </div>
    <div
      v-if="loadFailed"
      class="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"
      role="status"
    >
      {{ $st('The installed plugin list is unavailable.') }}
      <Button type="button" size="sm" variant="ghost" @click="emit('retry')">{{ $st('Retry') }}</Button>
    </div>
    <ul
      v-if="rows.length"
      class="max-h-64 divide-y divide-border/50 overflow-auto rounded-md border border-border/60"
      :aria-label="$st('Plugin access')"
    >
      <li
        v-for="row in rows"
        :key="row.id"
        class="flex min-w-0 items-center justify-between gap-2 px-2.5 py-1.5"
        :data-permission-plugin="row.id"
      >
        <div class="min-w-0">
          <p class="break-all text-sm leading-tight" :title="row.id">{{ row.title }}</p>
          <p v-if="row.title !== row.id || row.runtimeState" class="text-xs text-muted-foreground">
            <span v-if="row.title !== row.id" class="break-all font-mono">{{ row.id }}</span>
            <span v-if="row.title !== row.id && row.runtimeState" aria-hidden="true"> · </span>
            {{ pluginStateLabel(row.runtimeState) }}
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <span class="text-xs text-muted-foreground">{{ row.enabled ? $st('Enabled') : $st('Disabled') }}</span>
          <SettingsToggle
            v-if="editable"
            :model-value="row.enabled"
            :label="row.title"
            :disabled="disabled"
            @update:model-value="emit('change', row.id, $event)"
          />
          <div v-if="editable" class="w-6">
            <IconButton
              v-if="row.overridden"
              type="button"
              size="xs"
              :tooltip="$st('Use inherited value')"
              :aria-label="`${$st('Use inherited value')} · ${row.title}`"
              :disabled="disabled"
              @click="emit('change', row.id, undefined)"
              ><RiResetLeftLine class="h-3.5 w-3.5"
            /></IconButton>
          </div>
        </div>
      </li>
    </ul>
    <p v-else-if="!loadFailed" class="py-2 text-xs text-muted-foreground">{{ $st('No matching plugins.') }}</p>
  </div>
</template>
