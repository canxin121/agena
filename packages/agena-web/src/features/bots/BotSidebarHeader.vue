<script setup lang="ts">
import { RiAddLine, RiRefreshLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'

const { t } = useI18n()

const props = withDefaults(
  defineProps<{
    query: string
    loading: boolean
    isTouchPointer?: boolean
    isCompactTouch?: boolean
  }>(),
  {
    isTouchPointer: false,
    isCompactTouch: false,
  },
)

const emit = defineEmits<{
  (e: 'update:query', v: string): void
  (e: 'create'): void
  (e: 'refresh'): void
}>()
</script>

<template>
  <div class="h-9 pt-1 select-none pl-3.5 pr-2 flex-shrink-0">
    <div class="flex h-full items-center justify-between gap-2">
      <div class="min-w-0 flex items-center gap-2">
        <p class="typography-ui-label font-medium text-muted-foreground">{{ t('bot.title') }}</p>
      </div>

      <div class="flex items-center gap-1">
        <IconButton
          data-testid="sidebar-bot-create"
          :tooltip="String(t('bot.profiles.create'))"
          :title="String(t('bot.profiles.create'))"
          :aria-label="String(t('bot.profiles.create'))"
          :is-touch-pointer="props.isTouchPointer || props.isCompactTouch"
          @click="emit('create')"
        >
          <RiAddLine class="h-4 w-4" />
        </IconButton>

        <IconButton
          data-testid="sidebar-bot-refresh"
          :tooltip="String(t('common.refresh'))"
          :title="String(t('common.refresh'))"
          :aria-label="String(t('common.refresh'))"
          :is-touch-pointer="props.isTouchPointer || props.isCompactTouch"
          :disabled="loading"
          @click="emit('refresh')"
        >
          <RiRefreshLine class="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  </div>

  <div class="px-3 py-2 flex-shrink-0">
    <SearchInput
      :model-value="query"
      @update:model-value="(v) => emit('update:query', String(v || ''))"
      @clear="emit('update:query', '')"
      :placeholder="String(t('bot.profiles.search'))"
      class="text-xs"
      input-class="h-8 text-xs"
      :input-aria-label="String(t('bot.profiles.search'))"
      :input-title="String(t('bot.profiles.search'))"
      :search-aria-label="String(t('common.search'))"
      :search-title="String(t('common.search'))"
      :clear-aria-label="String(t('common.clear'))"
      :clear-title="String(t('common.clear'))"
      :is-touch-pointer="props.isTouchPointer || props.isCompactTouch"
    />
  </div>
</template>
