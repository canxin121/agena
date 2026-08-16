<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import SettingsSidebarNavRow from './SettingsSidebarNavRow.vue'
import type { SettingsSidebarRenderGroup, SettingsTab } from './settingsSidebarNavigation'

const props = withDefaults(
  defineProps<{
    groups?: SettingsSidebarRenderGroup[]
  }>(),
  {
    groups: () => [],
  },
)

const emit = defineEmits<{
  (e: 'navigate-tab', id: SettingsTab): void
}>()

const { t } = useI18n()

const visibleGroups = computed(() => props.groups.filter((group) => group.items.length > 0))
const hasRows = computed(() => visibleGroups.value.length > 0)
</script>

<template>
  <div class="min-h-0 overflow-x-hidden">
    <div v-if="!hasRows" class="px-4 py-8 text-center text-muted-foreground">
      <div class="typography-ui-label font-semibold">{{ t('common.noOptionsFound') }}</div>
    </div>

    <div v-else class="space-y-2 pb-2 pl-2 pr-1">
      <div v-for="(group, groupIndex) in visibleGroups" :key="group.id">
        <div v-if="groupIndex > 0" class="mx-1 my-2 border-t border-sidebar-border/60" />

        <div class="space-y-0.5">
          <div v-for="row in group.items" :key="row.id">
            <SettingsSidebarNavRow
              :label="row.label"
              :active="row.active"
              :icon="row.icon"
              @click="emit('navigate-tab', row.id)"
            />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
