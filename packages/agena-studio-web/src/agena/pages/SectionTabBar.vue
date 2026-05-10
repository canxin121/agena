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
  <div v-if="props.tabs.length" class="button-row" style="margin-bottom: 16px; flex-wrap: wrap">
    <button
      v-for="tab in props.tabs"
      :key="tab.id"
      class="button"
      :class="{ primary: props.activeTab === tab.id }"
      @click="emit('update:activeTab', tab.id)"
    >
      {{ tab.label }}
    </button>
  </div>
</template>
