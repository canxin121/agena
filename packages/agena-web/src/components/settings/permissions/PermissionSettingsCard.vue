<script setup lang="ts">
import type { Component } from 'vue'
import SettingsCard from '../SettingsCard.vue'
defineProps<{ title: string; description?: string; count?: number; icon?: Component }>()
</script>

<template>
  <SettingsCard :title="title" :description="description" :count="count" :icon="icon" data-permission-card>
    <template v-if="$slots.actions" #actions><slot name="actions" /></template>
    <slot />
  </SettingsCard>
</template>
