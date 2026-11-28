<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiAddLine,
  RiArrowDownSLine,
  RiArrowRightSLine,
  RiRefreshLine,
} from '@remixicon/vue'

import IconButton from '@/components/ui/IconButton.vue'
import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import { directoryEntryLabel } from '@/features/sessions/model/labels'
import type { DirectoryEntry } from '@/features/sessions/model/types'
import type { AcpDirectoryActivityState } from './acpSidebarModel'

const { t } = useI18n()

const props = withDefaults(
  defineProps<{
    directory: DirectoryEntry
    uiIsCompactLayout: boolean
    collapsed?: boolean
    focused?: boolean
    activityState?: AcpDirectoryActivityState
    loading?: boolean
    creatingSession?: boolean
    canCreate?: boolean
  }>(),
  {
    collapsed: true,
    focused: false,
    activityState: null,
    loading: false,
    creatingSession: false,
    canCreate: true,
  },
)

const emit = defineEmits<{
  (e: 'toggle-collapse'): void
  (e: 'refresh'): void
  (e: 'new-session'): void
}>()

const activityTitle = computed(() => {
  if (props.activityState === 'running') {
    return String(t('chat.sidebar.directoriesList.activity.runningTooltip'))
  }
  if (props.activityState === 'attention') {
    return String(t('chat.sidebar.directoriesList.activity.attentionTooltip'))
  }
  if (props.activityState === 'mixed') {
    return String(t('chat.sidebar.directoriesList.activity.mixedTooltip'))
  }
  return ''
})

const activityDotClass = computed(() => {
  if (props.activityState === 'running') {
    return 'bg-primary animate-pulse'
  }
  if (props.activityState === 'attention') {
    return 'bg-amber-500'
  }
  if (props.activityState === 'mixed') {
    return 'bg-amber-500 animate-pulse ring-1 ring-primary/40 ring-offset-1 ring-offset-sidebar'
  }
  return ''
})
</script>

<template>
  <SidebarListItem
    :data-keyboard-item="`directory:${directory.id}`"
    :data-keyboard-label="directoryEntryLabel(directory)"
    data-keyboard-level="1"
    :data-keyboard-expanded="String(!collapsed)"
    :data-keyboard-selected="String(focused)"
    :active="focused"
    :actions-always-visible="uiIsCompactLayout"
    class="relative gap-1.5"
    @click="emit('toggle-collapse')"
  >
    <template #icon>
      <button
        type="button"
        data-keyboard-toggle
        class="h-3.5 w-3.5 flex-shrink-0 inline-flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:dark:bg-accent/40 hover:bg-primary/6 cursor-pointer active:scale-95 transition"
        :aria-label="
          collapsed
            ? String(t('chat.sidebar.directoriesList.expandDirectory'))
            : String(t('chat.sidebar.directoriesList.collapseDirectory'))
        "
        @click.stop="emit('toggle-collapse')"
      >
        <RiArrowRightSLine v-if="collapsed" class="h-3 w-3" />
        <RiArrowDownSLine v-else class="h-3 w-3" />
      </button>
    </template>

    <div class="flex min-w-0 flex-col justify-center gap-0.5 py-px">
      <div
        class="typography-ui-label min-w-0 pb-px text-left leading-[1.4]"
        :class="focused ? 'text-primary' : 'text-foreground'"
      >
        <div class="flex min-w-0 items-center gap-2">
          <div class="min-w-0 flex-1 truncate">{{ directoryEntryLabel(directory) }}</div>
          <span
            v-if="activityState"
            class="inline-flex items-center flex-shrink-0"
            :title="activityTitle"
            :aria-label="activityTitle"
          >
            <span class="inline-flex h-1.5 w-1.5 rounded-full" :class="activityDotClass" />
            <span class="sr-only">{{ activityTitle }}</span>
          </span>
        </div>
      </div>
      <div class="typography-micro min-w-0 truncate pb-px font-mono text-left leading-tight text-muted-foreground/60">
        {{ directory.path }}
      </div>
    </div>

    <template #actions>
      <IconButton
        size="xs"
        class="text-muted-foreground hover:text-foreground hover:dark:bg-accent/40 hover:bg-primary/6"
        :title="String(t('chat.sidebar.directoryActions.refresh.label'))"
        :aria-label="String(t('chat.sidebar.directoryActions.refresh.label'))"
        :disabled="loading"
        @click.stop="emit('refresh')"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>

      <IconButton
        size="xs"
        class="text-muted-foreground hover:text-primary hover:dark:bg-accent/40 hover:bg-primary/6"
        :title="String(t('chat.sidebar.directoryActions.newSession.label'))"
        :aria-label="String(t('chat.sidebar.directoryActions.newSession.label'))"
        :disabled="!canCreate || creatingSession"
        @click.stop="emit('new-session')"
      >
        <RiAddLine class="h-4 w-4" />
      </IconButton>
    </template>
  </SidebarListItem>
</template>
