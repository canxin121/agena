<script setup lang="ts">
import { DialogDescription, DialogTitle } from 'radix-vue'
import { RiCloseLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import IconButton from '@/components/ui/IconButton.vue'
import { useUiStore } from '@/stores/ui'

defineProps<{ title?: string; description?: string }>()
defineEmits<{ close: [] }>()
const { t } = useI18n()
const ui = useUiStore()
</script>

<template>
  <header class="flex shrink-0 items-start gap-2 border-b border-border/50 px-3 py-2 sm:px-4">
    <IconButton
      variant="ghost"
      size="sm"
      class="shrink-0"
      :class="ui.isTouchPointer ? 'h-10 w-10' : 'h-8 w-8'"
      :tooltip="t('common.close')"
      :is-touch-pointer="ui.isTouchPointer"
      :aria-label="t('common.close')"
      @click="$emit('close')"
    >
      <RiCloseLine class="h-4 w-4" />
    </IconButton>
    <div class="min-w-0 flex-1 py-1.5">
      <DialogTitle v-if="title" class="text-sm font-semibold leading-5 text-foreground [overflow-wrap:anywhere]">
        {{ title }}
      </DialogTitle>
      <DialogDescription
        v-if="description"
        class="mt-0.5 text-xs leading-5 text-muted-foreground [overflow-wrap:anywhere]"
      >
        {{ description }}
      </DialogDescription>
    </div>
  </header>
</template>
