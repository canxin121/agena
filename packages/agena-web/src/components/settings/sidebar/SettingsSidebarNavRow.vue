<script setup lang="ts">
import { computed } from 'vue'
import {
  RiArrowRightSLine,
  RiBugLine,
  RiComputerLine,
  RiFlowChart,
  RiPlugLine,
  RiSettingsLine,
  RiShieldCheckLine,
} from '@remixicon/vue'

import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import type { SettingsSidebarIconKey } from './settingsSidebarNavigation'

const props = withDefaults(
  defineProps<{
    label: string
    active?: boolean
    branchActive?: boolean
    icon?: SettingsSidebarIconKey
    depth?: number
    hasChildren?: boolean
    expanded?: boolean
  }>(),
  {
    active: false,
    branchActive: false,
    icon: undefined,
    depth: 0,
    hasChildren: false,
    expanded: false,
  },
)

const emit = defineEmits<{
  (e: 'click', event: MouseEvent): void
}>()

const topLevelIcon = computed(() => {
  if (props.icon === 'models-providers') return RiComputerLine
  if (props.icon === 'permissions') return RiShieldCheckLine
  if (props.icon === 'plugins-tools') return RiPlugLine
  if (props.icon === 'runtime-session') return RiFlowChart
  if (props.icon === 'diagnostics') return RiBugLine
  if (props.icon === 'interface') return RiSettingsLine
  return null
})

const indent = computed(() => (props.depth > 0 ? 28 + (props.depth - 1) * 16 : undefined))
</script>

<template>
  <SidebarListItem
    :active="active"
    :indent="indent"
    :density="depth > 0 ? 'compact' : 'default'"
    class="gap-1.5"
    :class="branchActive && !active ? 'text-foreground' : ''"
    :aria-expanded="hasChildren ? expanded : undefined"
    @click="emit('click', $event)"
  >
    <template #icon>
      <div class="flex items-center gap-1.5">
        <RiArrowRightSLine
          v-if="hasChildren"
          class="h-3.5 w-3.5 transition-transform duration-150"
          :class="expanded ? 'rotate-90 text-foreground/80' : 'text-muted-foreground/70'"
        />
        <span v-else-if="depth === 0" class="block h-3.5 w-3.5" />
        <span
          v-else
          class="mx-1 inline-flex h-1.5 w-1.5 rounded-full"
          :class="active ? 'bg-primary' : 'bg-muted-foreground/45'"
        />

        <component
          :is="topLevelIcon"
          v-if="topLevelIcon"
          class="h-4 w-4"
          :class="branchActive ? 'text-primary' : 'text-muted-foreground/80'"
        />
      </div>
    </template>

    <div class="flex min-w-0 flex-col justify-center gap-0.5 py-px">
      <div
        class="typography-ui-label min-w-0 truncate leading-[1.4]"
        :class="active || branchActive ? 'text-foreground' : 'text-foreground/90'"
      >
        {{ label }}
      </div>
    </div>
  </SidebarListItem>
</template>
