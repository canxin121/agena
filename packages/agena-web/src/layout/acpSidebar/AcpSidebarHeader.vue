<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiRefreshLine, RiSettings3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import type { PickerOption } from '@/components/ui/pickerOption.types'
import { formatAcpCommandPreview } from '@/lib/acpAgentsConfig'
import type { ExternalAgent, ExternalAgentCatalogEntry } from '@/types/chat'

const { t } = useI18n()

const props = withDefaults(
  defineProps<{
    agents: readonly ExternalAgentCatalogEntry[]
    agent: ExternalAgent
    query: string
    loading: boolean
    isTouchPointer?: boolean
    showSearch?: boolean
  }>(),
  {
    isTouchPointer: false,
    showSearch: true,
  },
)

const emit = defineEmits<{
  (e: 'update:agent', value: ExternalAgent): void
  (e: 'update:query', value: string): void
  (e: 'add-directory'): void
  (e: 'refresh'): void
  (e: 'open-settings'): void
}>()

const showAgentPills = computed(() => props.agents.length > 0 && props.agents.length <= 4)

const agentOptions = computed<PickerOption[]>(() =>
  props.agents.map((entry) => {
    const commandPreview = formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id
    return {
      value: entry.id,
      label: entry.name,
      description:
        entry.available === false
          ? `${t('chat.nativeAgent.installGuide.notInstalledBadge')} · ${commandPreview}`
          : commandPreview,
      group: String(t(entry.builtin ? 'settings.acpAgents.status.builtin' : 'settings.acpAgents.status.custom')),
      keywords: `${entry.id} ${entry.name} ${entry.command}`,
    }
  }),
)
</script>

<template>
  <div class="h-9 pt-1 select-none pl-3.5 pr-2 flex-shrink-0">
    <div class="flex h-full items-center justify-between gap-2">
      <div class="min-w-0 flex items-center gap-2">
        <p class="typography-ui-label font-medium text-muted-foreground">
          {{ t('nav.acp') }}
        </p>
      </div>

      <div class="flex items-center gap-1">
        <IconButton
          :tooltip="String(t('chat.sidebar.header.addDirectory'))"
          :title="String(t('chat.sidebar.header.addDirectory'))"
          :aria-label="String(t('chat.sidebar.header.addDirectory'))"
          :is-touch-pointer="isTouchPointer"
          @click="emit('add-directory')"
        >
          <RiAddLine class="h-4 w-4" />
        </IconButton>

        <IconButton
          :tooltip="String(t('chat.sidebar.header.refresh'))"
          :title="String(t('chat.sidebar.header.refresh'))"
          :aria-label="String(t('chat.sidebar.header.refresh'))"
          :is-touch-pointer="isTouchPointer"
          :disabled="loading"
          @click="emit('refresh')"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>

        <IconButton
          :tooltip="String(t('chat.nativeAgent.configureAgents'))"
          :title="String(t('chat.nativeAgent.configureAgents'))"
          :aria-label="String(t('chat.nativeAgent.configureAgents'))"
          :is-touch-pointer="isTouchPointer"
          @click="emit('open-settings')"
        >
          <RiSettings3Line class="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  </div>

  <div class="px-3 pt-1 pb-1.5 flex-shrink-0">
    <div
      v-if="showAgentPills"
      role="radiogroup"
      :aria-label="t('chat.nativeAgent.agent')"
      class="flex flex-wrap gap-0.5 rounded-md border border-sidebar-border/70 bg-sidebar-accent/30 p-0.5"
    >
      <button
        v-for="entry in agents"
        :key="entry.id"
        type="button"
        role="radio"
        :aria-checked="agent === entry.id"
        :title="
          entry.available === false
            ? `${t('chat.nativeAgent.installGuide.notInstalledBadge')} · ${formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id}`
            : formatAcpCommandPreview(entry.command, entry.args ?? []) || entry.id
        "
        class="flex h-7 min-w-0 flex-1 basis-[calc(50%-0.125rem)] items-center justify-center gap-1.5 rounded px-2 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="
          agent === entry.id
            ? 'bg-background text-foreground shadow-xs ring-1 ring-border/60'
            : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground'
        "
        @click="emit('update:agent', entry.id)"
      >
        <span
          v-if="entry.available === false"
          class="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500"
          aria-hidden="true"
        />
        <span
          v-else-if="!entry.builtin"
          class="h-1.5 w-1.5 shrink-0 rounded-full bg-primary"
          aria-hidden="true"
        />
        <span class="truncate">{{ entry.name }}</span>
      </button>
    </div>

    <OptionPicker
      v-else-if="agents.length > 4"
      :model-value="agent"
      :options="agentOptions"
      :title="t('chat.nativeAgent.agent')"
      :placeholder="t('chat.nativeAgent.agent')"
      :search-placeholder="t('chat.nativeAgent.searchAgents')"
      :include-empty="false"
      size="sm"
      trigger-class="w-full"
      @update:model-value="emit('update:agent', String($event || ''))"
    />

    <div
      v-else
      class="space-y-2 rounded-md border border-dashed border-border/80 bg-muted/20 p-2.5 text-xs text-muted-foreground"
    >
      <p>{{ t('chat.nativeAgent.noEnabledAgents') }}</p>
      <Button size="sm" variant="outline" class="w-full" @click="emit('open-settings')">
        <RiSettings3Line class="mr-1.5 h-3.5 w-3.5" />
        {{ t('chat.nativeAgent.configureAgents') }}
      </Button>
    </div>
  </div>

  <div v-if="showSearch" class="px-3 pb-2 flex-shrink-0">
    <SearchInput
      :model-value="query"
      @update:model-value="(v) => emit('update:query', String(v || ''))"
      @clear="emit('update:query', '')"
      :placeholder="String(t('chat.sidebar.header.searchPlaceholder'))"
      class="text-xs"
      input-class="h-8 text-xs"
      :input-aria-label="String(t('chat.sidebar.header.searchAria'))"
      :input-title="String(t('chat.sidebar.header.searchAria'))"
      :search-aria-label="String(t('common.search'))"
      :search-title="String(t('common.search'))"
      :clear-aria-label="String(t('chat.sidebar.header.clearSearch'))"
      :clear-title="String(t('common.clear'))"
      :is-touch-pointer="isTouchPointer"
    />
  </div>
</template>
