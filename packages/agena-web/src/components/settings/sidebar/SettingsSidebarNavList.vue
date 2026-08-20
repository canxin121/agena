<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import SettingsSidebarNavRow from './SettingsSidebarNavRow.vue'
import type { SettingsSidebarRenderGroup, SettingsSidebarTabRow } from './settingsSidebarNavigation'

const props = withDefaults(
  defineProps<{
    groups?: SettingsSidebarRenderGroup[]
  }>(),
  {
    groups: () => [],
  },
)

const emit = defineEmits<{
  (e: 'activate-row', row: SettingsSidebarTabRow): void
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

        <div class="px-3 pb-1 pt-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/70">
          {{ t(group.labelKey) }}
        </div>

        <div class="space-y-0.5">
          <SettingsSidebarNavRow
            v-for="row in group.items"
            :key="row.key"
            :label="row.label"
            :active="row.active"
            :branch-active="row.branchActive"
            :icon="row.icon"
            :depth="row.depth"
            :has-children="row.hasChildren"
            :expanded="row.expanded"
            @click="emit('activate-row', row)"
          />
        </div>
      </div>
    </div>
  </div>
</template>
