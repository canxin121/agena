<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RiRobot2Line } from '@remixicon/vue'
import { botAvatarSource } from './avatar'

const props = withDefaults(
  defineProps<{
    name?: string
    avatar?: string | null
    size?: 'xs' | 'sm' | 'md' | 'lg' | 'xl'
  }>(),
  { name: '', avatar: null, size: 'md' },
)
const sizes = {
  xs: 'h-5 w-5 rounded-md text-[8px]',
  sm: 'h-6 w-6 rounded-md text-[10px]',
  md: 'h-8 w-8 rounded-lg text-xs',
  lg: 'h-12 w-12 rounded-2xl text-sm',
  xl: 'h-16 w-16 rounded-2xl text-lg',
}
const source = computed(() => botAvatarSource(props.avatar))
const failedSource = ref<string | null>(null)
watch(source, () => {
  failedSource.value = null
})
const showImage = computed(() => source.value && source.value !== failedSource.value)
const initials = computed(() => Array.from(props.name.trim()).slice(0, 2).join('').toLocaleUpperCase())

function imageFailed(event: Event) {
  const failed = (event.target as HTMLImageElement).getAttribute('src')
  if (failed === source.value) failedSource.value = failed
}
</script>

<template>
  <span
    class="inline-flex shrink-0 items-center justify-center overflow-hidden bg-primary/10 font-semibold text-primary"
    :class="sizes[size]"
    :data-bot-avatar="showImage ? 'image' : 'fallback'"
    aria-hidden="true"
  >
    <img
      v-if="showImage"
      :key="source!"
      :src="source!"
      alt=""
      decoding="async"
      class="h-full w-full object-cover"
      @error="imageFailed"
    />
    <span v-else-if="initials">{{ initials }}</span>
    <RiRobot2Line v-else class="h-1/2 w-1/2" />
  </span>
</template>
