<script setup lang="ts">
import { RiLoader4Line } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
</script>

<template>
  <span
    class="inline-flex items-center gap-2 text-xs font-normal text-muted-foreground"
    role="status"
    aria-live="polite"
    data-part-loading
  >
    <RiLoader4Line class="h-4 w-4 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
    {{ t('common.loading') }}
  </span>
</template>
