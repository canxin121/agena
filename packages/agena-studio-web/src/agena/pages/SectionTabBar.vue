<script setup lang="ts">
const props = defineProps<{
  activeTab: string
  tabs: ReadonlyArray<{
    id: string
    label: string
  }>
}>()

const emit = defineEmits<{
  'update:activeTab': [value: string]
}>()
</script>

<template>
  <div v-if="props.tabs.length" class="section-tabs" role="tablist" aria-label="Section navigation">
    <button
      v-for="tab in props.tabs"
      :key="tab.id"
      class="section-tab"
      :class="{ active: props.activeTab === tab.id }"
      role="tab"
      :aria-selected="props.activeTab === tab.id"
      @click="emit('update:activeTab', tab.id)"
    >
      {{ tab.label }}
    </button>
  </div>
</template>
