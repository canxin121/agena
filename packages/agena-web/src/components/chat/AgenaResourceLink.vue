<script setup lang="ts">
import { computed, type Component } from 'vue'
import { RiExternalLinkLine, RiLinksLine } from '@remixicon/vue'

const props = defineProps<{ href: string; label: string; description?: string; icon?: Component }>()
const destination = computed(() => props.href.trim())
const fullTitle = computed(() => [...new Set([props.label, props.description].filter(Boolean))].join(' — '))
</script>

<template>
  <a
    v-if="destination"
    tabindex="0"
    :href="destination"
    target="_blank"
    rel="noopener noreferrer"
    class="part-link interaction-width"
    :title="fullTitle"
    :aria-label="label"
    :aria-description="description || undefined"
  >
    <span class="interaction-choice-marker" aria-hidden="true"><component :is="icon || RiLinksLine" /></span>
    <span class="interaction-choice-copy">
      <span class="interaction-choice-label">{{ label }}</span>
      <span v-if="description && description !== label" class="interaction-choice-description">{{ description }}</span>
    </span>
    <RiExternalLinkLine class="interaction-choice-arrow" aria-hidden="true" />
  </a>
  <div v-else class="min-w-0">
    <p class="break-words text-[13px] font-medium">{{ label }}</p>
    <p v-if="description && description !== label" class="mt-1 break-words text-xs text-muted-foreground">
      {{ description }}
    </p>
  </div>
</template>
