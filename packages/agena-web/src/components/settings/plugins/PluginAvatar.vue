<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiPlugLine } from '@remixicon/vue'
import { pluginIconSource } from '@/lib/pluginIcon'

const props = defineProps<{ icon?: string | null }>()
const source = computed(() => pluginIconSource(props.icon))
const failedSource = ref<string>()
watch(source, () => {
  failedSource.value = undefined
})

function onImageError(event: Event) {
  const failed = (event.currentTarget as HTMLImageElement).getAttribute('src')
  if (failed === source.value) failedSource.value = failed
}
</script>

<template>
  <span
    class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-primary/10 bg-primary/5 text-primary"
    aria-hidden="true"
    data-plugin-avatar
  >
    <img
      v-if="source && source !== failedSource"
      :key="source"
      :src="source"
      alt=""
      class="h-7 w-7 object-contain"
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
      data-plugin-icon
      @error="onImageError"
    />
    <RiPlugLine v-else class="h-5 w-5" data-plugin-icon-fallback />
  </span>
</template>
