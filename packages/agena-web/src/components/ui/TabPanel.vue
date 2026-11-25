<script setup lang="ts">
import type { Component } from 'vue'

withDefaults(defineProps<{ active: boolean; as?: string | Component }>(), { as: 'div' })
</script>

<template>
  <!-- Keep the ARIA target mounted without mounting inactive content. -->
  <component
    :is="as"
    v-show="active"
    class="focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
  >
    <slot v-if="active" />
  </component>
</template>
