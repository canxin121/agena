<script setup lang="ts">
import WorkspacePaneView from '@/layout/WorkspacePaneView.vue'

defineProps<{
  windowId?: string | null
}>()
</script>

<template>
  <section class="flex h-full min-h-0 flex-col bg-background">
    <div class="min-h-0 flex-1 overflow-hidden">
      <WorkspacePaneView v-if="windowId" :window-id="windowId" class="h-full" />
      <router-view v-else />
    </div>
  </section>
</template>
