<script setup lang="ts">
import { computed } from 'vue'
import { RiBugLine, RiComputerLine, RiFlowChart, RiPlugLine, RiShieldCheckLine, RiSettingsLine } from '@remixicon/vue'

import SidebarListItem from '@/components/ui/SidebarListItem.vue'
import type { SettingsSidebarIconKey } from './settingsSidebarNavigation'

const props = withDefaults(
  defineProps<{
    label: string
    active?: boolean
    icon?: SettingsSidebarIconKey
  }>(),
  {
    active: false,
    icon: 'interface',
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
  return RiSettingsLine
})
</script>

<template>
  <SidebarListItem :active="active" class="gap-1.5" @click="emit('click', $event)">
    <template #icon>
      <div class="flex items-center gap-1.5">
        <span class="inline-flex h-3.5 w-3.5 items-center justify-center rounded text-muted-foreground/70">
          <span class="block h-3.5 w-3.5" />
        </span>

        <component :is="topLevelIcon" class="h-4 w-4" :class="active ? 'text-primary' : 'text-muted-foreground/80'" />
      </div>
    </template>

    <div class="flex min-w-0 flex-col justify-center gap-0.5 py-px">
      <div
        class="typography-ui-label min-w-0 truncate leading-[1.4]"
        :class="active ? 'text-foreground' : 'text-foreground/90'"
      >
        {{ label }}
      </div>
    </div>
  </SidebarListItem>
</template>
