<script setup lang="ts">
import { useId } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiArrowDownSLine, RiArrowRightSLine } from '@remixicon/vue'
import SidebarPager from '@/layout/chatSidebar/components/SidebarPager.vue'
import SidebarSectionSkeleton from '@/layout/chatSidebar/components/SidebarSectionSkeleton.vue'
import { vKeyboardCollection } from '@/lib/keyboardCollection'

defineProps<{
  title: string
  open: boolean
  count: number
  page: number
  pageCount: number
  paging?: boolean
  loading?: boolean
  emptyMessage?: string
  toggleLabel?: string
  prevLabel?: string
  nextLabel?: string
  tree?: boolean
}>()
const emit = defineEmits<{ 'update:open': [open: boolean]; 'update:page': [page: number] }>()
const { t } = useI18n()
const bodyId = useId()
</script>

<template>
  <div
    v-keyboard-collection="{ tree, fallbackFocus: '[data-sidebar-section-toggle]' }"
    :role="tree ? 'tree' : 'group'"
    :aria-label="title"
    class="shrink-0 border-t border-sidebar-border/60 bg-sidebar/95"
  >
    <div
      class="flex min-h-10 min-w-0 items-center gap-2 rounded-md px-3 transition-colors"
      :class="open ? 'bg-secondary/30' : 'hover:bg-secondary/30 focus-within:bg-secondary/30'"
    >
      <button
        type="button"
        data-sidebar-section-toggle
        class="flex min-w-0 flex-1 items-center gap-2 self-stretch rounded-sm text-start focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        :aria-expanded="open"
        :aria-controls="open ? bodyId : undefined"
        :aria-label="toggleLabel || title"
        @click="emit('update:open', !open)"
      >
        <component
          :is="open ? RiArrowDownSLine : RiArrowRightSLine"
          class="size-4 shrink-0 text-muted-foreground"
          :class="!open && 'rtl:rotate-180'"
          aria-hidden="true"
        />
        <span class="typography-ui-label truncate font-medium text-muted-foreground">{{ title }}</span>
      </button>
      <span class="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground/80">{{ count }}</span>
      <SidebarPager
        v-if="open && pageCount > 1"
        class="shrink-0"
        :page="page"
        :page-count="pageCount"
        :disabled="paging"
        :prev-label="prevLabel || t('common.previousPage')"
        :next-label="nextLabel || t('common.nextPage')"
        @update:page="emit('update:page', $event)"
      />
    </div>
    <div v-if="open" :id="bodyId" class="space-y-1 px-2 pt-1 pb-2" :aria-busy="loading || paging || undefined">
      <SidebarSectionSkeleton v-if="loading" :rows="3" compact :with-header="false" />
      <p v-else-if="emptyMessage" class="px-2 py-2 text-xs text-muted-foreground">{{ emptyMessage }}</p>
      <slot v-else />
    </div>
  </div>
</template>
