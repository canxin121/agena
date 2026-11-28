<script setup lang="ts">
import { onScopeDispose, shallowRef, watch } from 'vue'
import { createBlobUrlRegistry } from '@/lib/blobUrlRegistry'
import type { McpAppMedia } from './media'

const props = defineProps<{ media: McpAppMedia[] }>()
const urls = createBlobUrlRegistry()
const previews = shallowRef<(McpAppMedia & { url?: string })[]>([])
watch(
  () => props.media,
  (files) => {
    urls.dispose()
    previews.value = files.map((file) => {
      try {
        return { ...file, url: urls.create(file.blob) }
      } catch {
        return file
      }
    })
  },
  { immediate: true },
)
onScopeDispose(() => urls.dispose())
</script>

<template>
  <ul class="mt-2 grid max-h-64 gap-2 overflow-auto" data-mcp-app-media>
    <li v-for="file in previews" :key="file.filename" class="min-w-0 rounded border p-2">
      <div class="break-all text-muted-foreground">{{ file.filename }}</div>
      <img
        v-if="file.url && file.kind === 'image'"
        :src="file.url"
        :alt="file.filename"
        class="mt-1 max-h-40 max-w-full"
      />
      <audio
        v-if="file.url && file.kind === 'audio'"
        :src="file.url"
        :aria-label="file.filename"
        controls
        preload="none"
        class="mt-1 w-full"
      />
    </li>
  </ul>
</template>
